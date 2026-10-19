//! `bagholder outside-debts-check <data folder> <out folder>`
//! (`docs/plans/debts-held-outside-wealthsimple.md`, brief 68): the one capture
//! the person supplies. With the folder's saved sign-in, used as it is and never
//! renewed (their app holds the same one), it asks Wealthsimple's net worth
//! feature once (a read), writes its reply anonymised to the out folder for
//! the repository, and prints counts only, never a name or an amount:
//! - each kind of outside account and the sign each debt's balance is stated with,
//!   which settles how a balance owed is read;
//! - how many of the book's bill payments and pre-authorized debits the matching
//!   settles, how many it would ask about, and how many it leaves as no debt.
//!
//! The book is never opened: a copy is made with SQLite's `VACUUM INTO` (which
//! reads only) and the counts are worked out on the copy, then it is deleted.

use std::collections::BTreeMap;
use std::path::Path;

use bagholder_core::json::Value;
use bagholder_wealthsimple::adapter::Source;

pub fn cli(args: &[String]) -> i32 {
    let [home, out] = args else {
        eprintln!("usage: bagholder outside-debts-check <data folder> <out folder>");
        return 2;
    };
    match run(Path::new(home), Path::new(out)) {
        Ok(text) => {
            print!("{text}");
            0
        }
        Err(e) => {
            eprintln!("the check failed: {e}");
            1
        }
    }
}

fn run(home: &Path, out: &Path) -> Result<String, String> {
    let net = bagholder_net::Net::new(std::sync::Arc::new(bagholder_net::SystemClock), std::sync::Arc::new(bagholder_net::Limiter::new()));
    let file = bagholder_wealthsimple::session::SessionFile { path: home.join("session.json") };
    // read-only: the person's app holds the same sign-in, which must never be renewed here
    let mut client = bagholder_wealthsimple::client::Client::read_only(&net, file);
    let reply = client.net_worth().map_err(|f| f.to_string())?.ok_or("Wealthsimple answered no net worth")?;
    let mut text = String::new();

    // the signs, from the reply as sent: kind -> (negative, zero, positive)
    let mut signs: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    let identity = bagholder_sources::reply::Node::root(&reply);
    let me = identity.text("id").map_err(|m| m.to_string())?.to_string();
    let mut others = 0;
    for e in identity.obj("netWorth").and_then(|n| n.list("externalFinancialEntities")).map_err(|m| m.to_string())? {
        if e.text("identityId").map_err(|m| m.to_string())? != me {
            others += 1;
            continue;
        }
        let kind = e.text("__typename").map_err(|m| m.to_string())?.trim_end_matches("ExternalFinancialEntity").to_string();
        let amount = e.obj("balance").and_then(|b| b.dec_text("amount")).map_err(|m| m.to_string())?;
        let s = signs.entry(kind).or_default();
        if amount.is_negative() {
            s.0 += 1;
        } else if amount.is_zero() {
            s.1 += 1;
        } else {
            s.2 += 1;
        }
    }
    text.push_str("Outside accounts by kind (negative / zero / positive balances):\n");
    for (kind, (n, z, p)) in &signs {
        let debt = if bagholder_wealthsimple::repayments::LIABILITY_TYPES.contains(&kind.as_str()) { "debt" } else { "asset" };
        text.push_str(&format!("  {kind} ({debt}): {n} / {z} / {p}\n"));
    }
    text.push_str(&format!("  another household member's: {others}\n"));

    // the reply for the repository, every identifying value replaced
    let mut a = bagholder_wealthsimple::anonymise::Anonymiser::default();
    let wrapped = bagholder_core::json::parse(&format!(r#"{{"data":{{"identity":{}}}}}"#, reply.canonical())).map_err(|e| e.to_string())?;
    let clean = a.value(&wrapped);
    let left = bagholder_wealthsimple::anonymise::leaks(&clean);
    if !left.is_empty() {
        return Err(format!("identifying values left at {left:?}: nothing was written"));
    }
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let path = out.join("FetchIdentityNetWorthAccounts-1.json");
    std::fs::write(&path, clean.canonical()).map_err(|e| format!("{}: {e}", path.display()))?;
    text.push_str(&format!("Anonymised reply: {}\n", path.display()));

    // the settle count, on a copy of the book
    let copy = std::env::temp_dir().join(format!("bagholder-outside-debts-check-{}", std::process::id()));
    std::fs::create_dir_all(&copy).map_err(|e| e.to_string())?;
    let counted = settle(home, &copy, &reply);
    // the copy goes whatever the count came to; one that cannot is said
    if let Err(e) = std::fs::remove_dir_all(&copy) {
        text.push_str(&format!("The book's copy in {} could not be removed ({e}): delete it by hand\n", copy.display()));
    }
    text.push_str(&counted?);
    Ok(text)
}

/// The settle count, worked out on a copy of the book made into `copy`.
fn settle(home: &Path, copy: &Path, reply: &Value) -> Result<String, String> {
    {
        let live = rusqlite::Connection::open_with_flags(home.join(bagholder_book::BOOK_FILE), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        live.execute("VACUUM INTO ?1", [copy.join(bagholder_book::BOOK_FILE).to_string_lossy().to_string()]).map_err(|e| e.to_string())?;
    }
    let now = bagholder_core::jiff::Timestamp::now();
    let (book, _) = bagholder_book::Book::open_in(copy, crate::app::APP_VERSION, now).map_err(|e| e.to_string())?;
    let conn = book.connections().map_err(|e| e.to_string())?.into_iter().next().ok_or("the book holds no Wealthsimple connection")?.id;
    let debts = bagholder_wealthsimple::read::outside_accounts(reply).map_err(|m| m.to_string())?;
    book.keep_outside_debts(conn, now, &debts).map_err(|e| e.to_string())?;
    let d = crate::engine_inputs::debts(&book)?;
    let payments = book
        .withdrawals_of("wealthsimple")
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|w| matches!(bagholder_core::json::parse(&w.payload), Ok(Value::Object(p)) if p.get("activity").and_then(bagholder_wealthsimple::repayments::payment_of).is_some()))
        .count();
    let asked: usize = d.asked.iter().map(|q| q.payments).sum();
    Ok(format!(
        "Bill payments and pre-authorized debits: {payments}\n  matched to a debt: {}\n  to ask about: {asked}, from {} payees\n  no debt: {}\n",
        d.repayments.len(),
        d.asked.len(),
        payments.saturating_sub(d.repayments.len() + asked)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count is worked out on a copy: the book's own file is unchanged, and
    /// each payment is counted once as matched, asked about or no debt.
    #[test]
    fn the_settle_count_is_worked_out_on_a_copy() {
        let (dir, book, _) = crate::engine_inputs::tests::with_payments();
        drop(book);
        let before = std::fs::read(dir.path().join(bagholder_book::BOOK_FILE)).unwrap();
        let reply = bagholder_core::json::parse(
            r#"{"id": "me", "netWorth": {"externalFinancialEntities": [
                {"__typename": "PersonalLoanExternalFinancialEntity", "id": "loan", "identityId": "me", "ownershipType": "individual", "visibility": {"household": false}, "displayName": "Loan", "institutionName": "anon-personal-229", "accountNumberLast4": null, "balance": {"amount": "1000", "currency": "CAD"}, "balanceAsOf": null},
                {"__typename": "CreditCardExternalFinancialEntity", "id": "card", "identityId": "me", "ownershipType": "individual", "visibility": {"household": false}, "displayName": "Card", "institutionName": "anon-personal-117", "accountNumberLast4": null, "balance": {"amount": "100", "currency": "CAD"}, "balanceAsOf": null},
                {"__typename": "LineOfCreditExternalFinancialEntity", "id": "loc", "identityId": "me", "ownershipType": "joint", "visibility": {"household": false}, "displayName": "LOC", "institutionName": "anon-personal-117", "accountNumberLast4": null, "balance": {"amount": "100", "currency": "CAD"}, "balanceAsOf": null}
            ]}}"#,
        )
        .unwrap();
        let copy = tempfile::tempdir().unwrap();
        let counted = settle(dir.path(), copy.path(), &reply).unwrap();
        assert_eq!(counted, "Bill payments and pre-authorized debits: 2\n  matched to a debt: 1\n  to ask about: 1, from 1 payees\n  no debt: 0\n", "an expired or unfinished payment moved no money and is not counted");
        assert_eq!(std::fs::read(dir.path().join(bagholder_book::BOOK_FILE)).unwrap(), before, "the book itself is untouched");
    }
}
