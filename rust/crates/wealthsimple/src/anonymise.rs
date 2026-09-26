//! Replies recorded from the owner's own account are committed only after every
//! value that identifies the owner is replaced by a stand-in
//! (`docs/plans/stage-3b-wealthsimple.md`, "Anonymised, not scrubbed of
//! figures"). Figures and tickers stay (owner, 2026-09-23).
//!
//! An identifier is replaced consistently: the same value becomes the same
//! stand-in everywhere it appears in one run, so a row still names the account it
//! belongs to and a leg the order it was part of.

use std::collections::BTreeMap;

use bagholder_core::json::Value;

/// Fields whose value identifies the owner, an account or a record of theirs.
/// A security's id (`sec-…`) is Wealthsimple's public id for the security and
/// stays.
pub const IDENTIFIERS: &[&str] = &[
    "id",
    "accountId",
    "canonicalAccountId",
    "identityId",
    "actorIdentityId",
    "opposingAccountId",
    "canonicalId",
    "externalCanonicalId",
    "activityCanonicalId",
    "orderBatchId",
    "externalId",
    "orderId",
    "groupId",
    "applicationFamilyId",
    "userReferenceId",
    "idempotencyKey",
    "externalReferenceId",
    // a page cursor spells the ids of the row it stops at
    "endCursor",
    "cursor",
];

/// Fields whose text is about the owner or the people they deal with.
pub const PERSONAL: &[&str] = &[
    "eTransferEmail",
    "eTransferName",
    "p2pHandle",
    "p2pMessage",
    "redactedExternalAccountNumber",
    "institutionName",
    "aftOriginatorName",
    "counterPartyName",
    "spendMerchant",
    "billPayCompanyName",
    "billPayPayeeNickname",
    "chequeNumber",
    "reference",
    "nickname",
    "productNickname",
    "title",
    "subtitle",
    "email",
    "legalName",
    "accountNickname",
    "clientCanonicalId",
];

/// Objects under these fields describe a person: every text in them is
/// personal, but the few that only classify.
pub const PEOPLE: &[&str] = &["accountOwners", "sentInvitations", "activeInvitation", "accountEntityRelationships"];

/// Texts inside a person's object that only classify, and stay.
const CLASSIFYING: &[&str] = &["ownershipType", "__typename", "status", "type", "role"];

/// The prefix every stand-in starts with.
pub const STAND_IN: &str = "anon-";

/// Whether a field's text value must be a stand-in.
pub fn must_be_stand_in(key: &str, text: &str) -> bool {
    PERSONAL.contains(&key) || (IDENTIFIERS.contains(&key) && !text.starts_with("sec-"))
}

/// Stand-ins handed out so far, kept across the replies of one run. One value is
/// one stand-in wherever it appears, under whichever field (an account's `id` in
/// the accounts list and `accountId` on its rows), so the replies still join.
#[derive(Default)]
pub struct Anonymiser {
    given: BTreeMap<String, String>,
    counts: BTreeMap<String, usize>,
}

impl Anonymiser {
    fn stand_in(&mut self, key: &str, text: &str) -> String {
        if let Some(s) = self.given.get(text) {
            return s.clone();
        }
        // named for what it is: a personal text, or an id's own kind word
        // (`order`, `tfsa`, `activity`), which says nothing about the owner
        let kind: String = if PERSONAL.contains(&key) {
            "personal".into()
        } else {
            let word: String = text.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
            if word.is_empty() || word.len() > 16 { "id".into() } else { word }
        };
        let n = self.counts.entry(kind.clone()).or_default();
        *n += 1;
        let s = format!("{STAND_IN}{kind}-{n}");
        self.given.insert(text.to_string(), s.clone());
        s
    }

    /// The value with every identifying text replaced.
    pub fn value(&mut self, v: &Value) -> Value {
        self.walk(v, None, false)
    }

    fn walk(&mut self, v: &Value, key: Option<&str>, person: bool) -> Value {
        match v {
            Value::String(t) if key.is_some_and(|k| must_be_stand_in(k, t) || (person && !CLASSIFYING.contains(&k))) => {
                let k = key.unwrap_or_default();
                Value::String(self.stand_in(if person && !IDENTIFIERS.contains(&k) { "email" } else { k }, t))
            }
            Value::Array(items) => Value::Array(items.iter().map(|i| self.walk(i, key, person)).collect()),
            Value::Object(map) => Value::Object(map.iter().map(|(k, item)| (k.clone(), self.walk(item, Some(k), person || PEOPLE.contains(&k.as_str())))).collect()),
            other => other.clone(),
        }
    }
}

/// Every place in a value where identifying text is not a stand-in: its path.
pub fn leaks(v: &Value) -> Vec<String> {
    fn walk(v: &Value, key: Option<&str>, person: bool, path: &str, out: &mut Vec<String>) {
        match v {
            Value::String(t) => {
                if let Some(k) = key {
                    if (must_be_stand_in(k, t) || (person && !CLASSIFYING.contains(&k))) && !t.starts_with(STAND_IN) {
                        out.push(path.to_string());
                    }
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    walk(item, key, person, &format!("{path}[{i}]"), out);
                }
            }
            Value::Object(map) => {
                for (k, item) in map {
                    walk(item, Some(k), person || PEOPLE.contains(&k.as_str()), &if path.is_empty() { k.clone() } else { format!("{path}.{k}") }, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, None, false, "", &mut out);
    out
}
