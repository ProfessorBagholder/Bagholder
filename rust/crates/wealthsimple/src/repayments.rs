//! Which outgoing payments repay a debt held outside Wealthsimple
//! (`docs/plans/debts-held-outside-wealthsimple.md`, "Matching a payment to a
//! liability"). A payment is matched to the outside account it went to: by the
//! person's own answer, by the account number's last four digits, or by the
//! institution the payment names. Where none of those names exactly one debt, the payment is unsettled
//! and the person is asked, once per payee. No amount is matched and no payee is
//! named here.

use std::collections::BTreeMap;

/// The kinds of outside entity that are debts, as Wealthsimple's net worth
/// feature types them (brief 23, the bundle's type map).
pub const LIABILITY_TYPES: [&str; 9] = ["CreditCard", "LineOfCredit", "Heloc", "Mortgage", "AutoLoan", "InstallmentLoan", "PersonalLoan", "StudentLoan", "OtherLiability"];

/// The purposes a pre-authorized debit states (`aftTransactionCategory`), as
/// Wealthsimple writes Payments Canada Standard 007's transaction codes: those
/// that name a loan or a mortgage. A debit with one of these is a debt's by
/// purpose; the institution it names says which debt.
pub const DEBT_PURPOSES: [&str; 14] = [
    "loans", "personal_loans", "dealer_plan_loans", "farm_improvement_loans", "home_improvement_loans", "term_loans", "insurance_loans", "commercial_loans", "student_loan",
    "mortgage", "residential_mortgage", "commercial_mortgage", "commercial_mortage", "farm_mortgage",
];

/// The purposes that name none: a debit with one of these (or with none stated)
/// may repay a debt, so the institution it names decides. Every other purpose
/// (insurance, utilities, taxes, rent, investments, payroll…) is not a debt's,
/// whoever pulled it (brief 67, 3).
pub const NO_PURPOSE: [&str; 5] = ["misc_payments", "bill_payment", "auto_payment", "business_pad", "expense_payment"];

/// A debt held outside Wealthsimple, as its net worth feature states it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Liability {
    pub id: String,
    pub kind: String,
    pub institution: String,
    pub last4: Option<String>,
}

/// What an outgoing payment states about where it went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payment {
    /// A bill payment: the company and the account number's redacted form.
    BillPay { company: String, account: Option<String> },
    /// A pre-authorized debit: who pulled it, and the purpose it states.
    Debit { originator: String, purpose: Option<String> },
}

impl Payment {
    /// The key a person's answer is kept under: the same payee, whatever the payment.
    pub fn payee_key(&self) -> String {
        match self {
            Payment::BillPay { company, account } => format!("bill:{}:{}", normalise(company), last4(account.as_deref().unwrap_or("")).unwrap_or_default()),
            Payment::Debit { originator, .. } => format!("debit:{}", normalise(originator)),
        }
    }
}

/// What a withdrawal row states about where it went: a bill payment or a
/// pre-authorized debit, read from the row as Wealthsimple sent it; none for
/// any other withdrawal (an EFT to the person's bank, an e-Transfer).
pub fn payment_of(row: &bagholder_core::json::Value) -> Option<Payment> {
    let bagholder_core::json::Value::Object(fields) = row else { return None };
    let text = |k: &str| match fields.get(k) {
        Some(bagholder_core::json::Value::String(v)) => Some(v.trim().to_string()).filter(|v| !v.is_empty()),
        _ => None,
    };
    if text("type").as_deref() != Some("WITHDRAWAL") {
        return None;
    }
    match text("subType").as_deref() {
        Some("BILL_PAY") => Some(Payment::BillPay { company: text("billPayCompanyName")?, account: text("redactedExternalAccountNumber") }),
        // the purpose as Wealthsimple states it: its category, else its type
        Some("AFT") => Some(Payment::Debit { originator: text("aftOriginatorName")?, purpose: text("aftTransactionCategory").or_else(|| text("aftTransactionType")) }),
        _ => None,
    }
}

/// How a payment was matched, or why it was not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Matched {
    /// It repaid this debt, by the rule named.
    Debt { liability: String, by: Rule },
    /// The person said it is not a debt.
    NotDebt,
    /// It may repay a debt and nothing says which: the person is asked.
    Unsettled { candidates: Vec<String> },
    /// Nothing about it suggests a debt (a utility, a subscription).
    NotOne,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Answer,
    AccountNumber,
    Institution,
}

/// The person's answer for a payee: the debt it repays, or none.
pub type Answers = BTreeMap<String, Option<String>>;

/// The last four digits a redacted account number ends with, if it ends with four.
pub fn last4(redacted: &str) -> Option<String> {
    let digits: Vec<char> = redacted.chars().filter(|c| c.is_ascii_digit()).collect();
    (digits.len() >= 4).then(|| digits[digits.len() - 4..].iter().collect())
}

/// A company's name as one institution's: case, accents and punctuation folded,
/// and the legal-form words dropped (Inc, Ltd, Ltée, Corp, LLC and the like).
/// Nothing else is dropped: "Bank of Canada" is part of a name (brief 67, 2).
pub fn normalise(name: &str) -> String {
    const LEGAL_FORMS: [&str; 15] = ["inc", "incorporated", "ltd", "ltee", "limited", "limitee", "corp", "corporation", "llc", "llp", "lp", "plc", "ulc", "sa", "co"];
    let folded: String = name
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ÿ' | 'ý' => 'y',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    folded.split_whitespace().filter(|w| !LEGAL_FORMS.contains(w)).collect::<Vec<_>>().join(" ")
}

/// The debt an outgoing payment repays, by the first rule that names exactly one.
pub fn match_payment(payment: &Payment, debts: &[Liability], answers: &Answers) -> Matched {
    if let Some(answer) = answers.get(&payment.payee_key()) {
        return match answer {
            Some(id) if debts.iter().any(|d| &d.id == id) => Matched::Debt { liability: id.clone(), by: Rule::Answer },
            // a debt answered for and since removed at Wealthsimple: asked again
            Some(_) => Matched::Unsettled { candidates: debts.iter().map(|d| d.id.clone()).collect() },
            None => Matched::NotDebt,
        };
    }
    let one = |found: Vec<&Liability>| -> Option<String> { (found.len() == 1).then(|| found[0].id.clone()) };
    match payment {
        Payment::BillPay { company, account } => {
            if let Some(digits) = account.as_deref().and_then(last4) {
                if let Some(id) = one(debts.iter().filter(|d| d.last4.as_deref() == Some(digits.as_str())).collect()) {
                    return Matched::Debt { liability: id, by: Rule::AccountNumber };
                }
            }
            // the company it was paid to is the institution of exactly one debt
            let at: Vec<&Liability> = debts.iter().filter(|d| same_institution(&d.institution, company)).collect();
            if let Some(id) = one(at.clone()) {
                return Matched::Debt { liability: id, by: Rule::Institution };
            }
            if !at.is_empty() {
                return Matched::Unsettled { candidates: unsettled(&at, debts) };
            }
            Matched::NotOne
        }
        Payment::Debit { originator, purpose } => {
            let purpose = purpose.as_deref();
            let for_debt = purpose.is_some_and(|p| DEBT_PURPOSES.contains(&p));
            // a debit stating another purpose (insurance, a utility, a tax) is not a
            // debt's, whoever pulled it
            if !for_debt && !purpose.is_none_or(|p| NO_PURPOSE.contains(&p)) {
                return Matched::NotOne;
            }
            let at: Vec<&Liability> = debts.iter().filter(|d| same_institution(&d.institution, originator)).collect();
            if let Some(id) = one(at.clone()) {
                return Matched::Debt { liability: id, by: Rule::Institution };
            }
            if !at.is_empty() || for_debt {
                return Matched::Unsettled { candidates: unsettled(&at, debts) };
            }
            Matched::NotOne
        }
    }
}

/// Whether a debt's institution and the name a payment states are one
/// institution: the whole names equal once folded, never one inside the other.
fn same_institution(institution: &str, stated: &str) -> bool {
    let (a, b) = (normalise(institution), normalise(stated));
    !a.is_empty() && a == b
}

/// The debts a person chooses among: those at the institution named, else every one.
fn unsettled(at: &[&Liability], debts: &[Liability]) -> Vec<String> {
    if at.is_empty() {
        debts.iter().map(|d| d.id.clone()).collect()
    } else {
        at.iter().map(|d| d.id.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn debt(id: &str, kind: &str, institution: &str, last4: Option<&str>) -> Liability {
        Liability { id: id.into(), kind: kind.into(), institution: institution.into(), last4: last4.map(String::from) }
    }

    fn bill(company: &str, account: &str) -> Payment {
        Payment::BillPay { company: company.into(), account: Some(account.into()) }
    }

    fn debit(originator: &str, purpose: Option<&str>) -> Payment {
        Payment::Debit { originator: originator.into(), purpose: purpose.map(String::from) }
    }

    fn debts() -> Vec<Liability> {
        vec![
            debt("card-a", "CreditCard", "Zeta Bank", Some("4821")),
            debt("loc-a", "LineOfCredit", "Zeta Bank", None),
            debt("mortgage-b", "Mortgage", "Omicron Trust", None),
            debt("card-c", "CreditCard", "Kappa Card Services", Some("1177")),
        ]
    }

    #[test]
    fn a_bill_payment_is_matched_by_its_account_numbers_last_four_digits() {
        assert_eq!(match_payment(&bill("Zeta Bank Visa", "****4821"), &debts(), &Answers::new()), Matched::Debt { liability: "card-a".into(), by: Rule::AccountNumber });
        assert_eq!(last4("xxxx-xxxx-xxxx-1177").as_deref(), Some("1177"));
        assert_eq!(last4("***12"), None, "fewer than four digits name nothing");
    }

    #[test]
    fn a_bill_payment_to_the_one_debt_at_its_company_is_that_debt() {
        assert_eq!(match_payment(&bill("KAPPA CARD SERVICES INC.", "****0000"), &debts(), &Answers::new()), Matched::Debt { liability: "card-c".into(), by: Rule::Institution });
        assert!(matches!(match_payment(&bill("Zeta Bank", "****0000"), &debts(), &Answers::new()), Matched::Unsettled { .. }), "two debts there: asked");
    }

    #[test]
    fn a_debit_is_matched_by_the_institution_it_names_whatever_the_legal_form_case_or_accents() {
        for originator in ["OMICRON TRUST", "Omicron Trust Inc.", "omicron trust ltd", "Omicron Trust Corporation", "Omicron Trust Ltée", "ÓMICRON TRUST, LLC"] {
            assert_eq!(match_payment(&debit(originator, Some("residential_mortgage")), &debts(), &Answers::new()), Matched::Debt { liability: "mortgage-b".into(), by: Rule::Institution }, "{originator}");
        }
    }

    #[test]
    fn a_name_inside_another_is_not_the_same_institution() {
        // substring traps: one name inside another, a shared first word, a dropped "Bank of Canada"
        let debts = vec![debt("mortgage-b", "Mortgage", "Omicron Trust", None), debt("loc-z", "LineOfCredit", "Zeta Bank of Canada", None)];
        for originator in ["Omicron", "Omicron Trust Company Holdings", "Omicron Trustees", "Zeta Bank", "Zeta"] {
            assert_eq!(match_payment(&debit(originator, Some("misc_payments")), &debts, &Answers::new()), Matched::NotOne, "{originator}");
        }
        assert_eq!(normalise("Zeta Bank of Canada"), "zeta bank of canada", "part of the name, kept");
    }

    #[test]
    fn a_debit_stating_another_purpose_is_never_a_repayment_by_name() {
        let insurance = debit("Omicron Trust", Some("life_insurance"));
        assert_eq!(match_payment(&insurance, &debts(), &Answers::new()), Matched::NotOne, "an insurance premium pulled by the lender");
        let loan = debit("Omicron Trust", Some("loans"));
        assert_eq!(match_payment(&loan, &debts(), &Answers::new()), Matched::Debt { liability: "mortgage-b".into(), by: Rule::Institution });
        assert_eq!(match_payment(&debit("Omicron Trust", None), &debts(), &Answers::new()), Matched::Debt { liability: "mortgage-b".into(), by: Rule::Institution }, "no purpose stated");
    }

    #[test]
    fn every_purpose_wealthsimple_states_is_a_debts_none_or_another_and_the_sets_are_its_own() {
        // the 130 purposes in Wealthsimple's bundle (`aftTransactionCategory`), Standard 007's codes
        let all: Vec<&str> = include_str!("debit_purposes.txt").lines().collect();
        assert_eq!(all.len(), 130);
        for p in DEBT_PURPOSES.iter().chain(NO_PURPOSE.iter()) {
            assert!(all.contains(p), "{p} is not a purpose Wealthsimple states");
        }
        let named_debt: Vec<&str> = all.iter().copied().filter(|p| p.contains("loan") || p.contains("mortgage") || p.contains("mortage")).filter(|p| !p.contains("insurance") || p.ends_with("_loans")).collect();
        let mut want = DEBT_PURPOSES.to_vec();
        want.sort();
        let mut got = named_debt.clone();
        got.sort();
        assert_eq!(got, want, "every loan or mortgage purpose, and no insurance on one");
    }

    #[test]
    fn a_withdrawal_row_says_where_it_went_and_nothing_else_does() {
        let row = |text: &str| payment_of(&bagholder_core::json::parse(text).unwrap());
        assert_eq!(row(r#"{"type": "WITHDRAWAL", "subType": "BILL_PAY", "billPayCompanyName": "Zeta Bank Visa", "redactedExternalAccountNumber": "****4821"}"#), Some(bill("Zeta Bank Visa", "****4821")));
        assert_eq!(row(r#"{"type": "WITHDRAWAL", "subType": "AFT", "aftOriginatorName": "Omicron Trust", "aftTransactionCategory": null, "aftTransactionType": "loans"}"#), Some(debit("Omicron Trust", Some("loans"))));
        assert_eq!(row(r#"{"type": "DEPOSIT", "subType": "AFT", "aftOriginatorName": "Payroll"}"#), None, "money in");
        assert_eq!(row(r#"{"type": "WITHDRAWAL", "subType": "E_TRANSFER"}"#), None);
        assert_eq!(row(r#"{"type": "WITHDRAWAL", "subType": "BILL_PAY", "billPayCompanyName": ""}"#), None, "a payment naming no one names no debt");
    }

    #[test]
    fn two_debts_at_one_institution_are_asked_and_the_answer_settles_later_payments() {
        let p = debit("ZETA BANK", Some("misc_payments"));
        let mut candidates = match match_payment(&p, &debts(), &Answers::new()) {
            Matched::Unsettled { candidates } => candidates,
            other => panic!("{other:?}"),
        };
        candidates.sort();
        assert_eq!(candidates, ["card-a", "loc-a"]);
        let answers: Answers = [(p.payee_key(), Some("loc-a".to_string()))].into();
        assert_eq!(match_payment(&debit("Zeta Bank Inc.", Some("misc_payments")), &debts(), &answers), Matched::Debt { liability: "loc-a".into(), by: Rule::Answer }, "the same payee, written another way");
        let none: Answers = [(p.payee_key(), None)].into();
        assert_eq!(match_payment(&p, &debts(), &none), Matched::NotDebt);
    }

    #[test]
    fn a_loan_debit_with_no_outside_debt_entered_is_asked_and_anything_else_is_not_a_debt() {
        assert!(matches!(match_payment(&debit("Unlisted Lender", Some("term_loans")), &debts(), &Answers::new()), Matched::Unsettled { .. }));
        assert_eq!(match_payment(&debit("City Hydro", Some("hydro_bill_payment")), &debts(), &Answers::new()), Matched::NotOne, "a utility");
        assert_eq!(match_payment(&bill("Rho Telecom", "****9999"), &debts(), &Answers::new()), Matched::NotOne, "a bill to no debt's institution");
    }

    #[test]
    fn an_answer_naming_a_debt_since_removed_is_asked_again() {
        let p = debit("Sigma Lending", Some("loans"));
        let answers: Answers = [(p.payee_key(), Some("gone".to_string()))].into();
        assert!(matches!(match_payment(&p, &debts(), &answers), Matched::Unsettled { .. }));
    }

}
