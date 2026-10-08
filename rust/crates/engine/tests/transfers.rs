//! A holding moved to or from an account of a type Bagholder does not know
//! (`SPEC.md` §2, Moves between your accounts): whether the move kept its cost
//! turns on the type, so both holdings wait on it, whichever side it is and
//! whatever the other account's plan. The case format builds only known
//! types, so this is held here.

mod common;

use serde_json::json;

use bagholder_core::account::AccountType;
use bagholder_engine::ledger::match_lots;
use common::*;

#[test]
fn a_move_with_an_account_of_an_unknown_type_waits_on_the_type_on_both_sides() {
    for registration in ["none", "tfsa", "resp", "rrsp", "group-rrsp", "rrif", "fhsa", "lira"] {
        for unknown in ["A", "B"] {
            let case = json!({
                "today": "2026-03-01",
                "accounts": [{"id": "A", "registration": registration}, {"id": "B", "registration": registration}],
                "instruments": [{"id": "X"}],
                "transactions": [
                    {"id": "t1", "account": "A", "day": "2026-01-05", "at": "2026-01-05T15:00:00Z", "kind": "buy", "instrument": "X", "qty": "10", "cash": "-100"},
                    {"id": "t2", "account": "A", "day": "2026-01-10", "at": "2026-01-10T15:00:00Z", "kind": "transfer-out", "instrument": "X", "qty": "-10", "value": "150"},
                    {"id": "t3", "account": "B", "day": "2026-01-10", "at": "2026-01-10T15:01:00Z", "kind": "transfer-in", "instrument": "X", "qty": "10"}
                ],
                "transfer_links": [{"out": "t2", "in": "t3"}],
            });
            let mut built = build(&case);
            let id = account_of(&built, unknown);
            built.inputs.ledger.accounts.get_mut(&id).unwrap().account.account_type = AccountType::Unrecognised("SOMETHING_NEW".into());
            let m = match_lots(&built.inputs);
            let x = built.inputs.ledger.instruments.keys().next().copied().unwrap();
            let b = account_of(&built, "B");
            let held = m.books.get(&(b, x)).expect("the units arrived in B");
            assert!(held.taint.has_word("registration-unknown"), "{registration}, {unknown} unknown: B's holding waits on the type: {:?}", held.taint);
            let a = account_of(&built, "A");
            assert!(m.books.get(&(a, x)).is_some_and(|h| h.taint.has_word("registration-unknown")), "{registration}, {unknown} unknown: A's holding waits on the type");
        }
    }
}

/// The book's id of the account a case calls `label`: the one its labelled
/// transactions are in.
fn account_of(built: &Built, label: &str) -> bagholder_core::AccountId {
    let t = match label {
        "A" => "t1",
        _ => "t3",
    };
    let id = &built.tx[t];
    built.inputs.ledger.transactions.iter().find(|x| &x.id == id).unwrap().account
}
