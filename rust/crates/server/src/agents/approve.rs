//! What the person does on their own page (`docs/plans/agents-from-anywhere.md`,
//! §3, §4.1): allow or deny a sign-in, make a key (shown once), and cut off a grant
//! or a key. Reached only from the main listener's routes (`http::agents`), behind
//! its gate, which no tunnel reaches; nothing on the agents' listener calls it.

use bagholder_book::agents::AgentsBook;
use bagholder_core::jiff::Timestamp;
use bagholder_diff_derive::Diff;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::secret;

/// A sign-in waiting on the person, as their page shows it.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct Waiting {
    pub id: String,
    pub code: String,
    /// Who is asking: the metadata document's host, or a registered client's redirect host.
    pub who: String,
    pub name: String,
    /// Whether the name was read from the agent's own published document (`true`),
    /// or given by the agent when it registered, unchecked.
    pub checked: bool,
    /// The host the agent is sent back to.
    pub redirect_host: String,
    pub made_at: String,
}

/// One agent that may read: a grant (signed in) or a key.
#[derive(Clone, Debug, PartialEq, Serialize, TS, Diff)]
#[serde(rename_all = "camelCase")]
#[diff(key = id)]
pub struct Connected {
    pub id: String,
    pub name: String,
    /// `sign-in`, `mcp-key` or `api-key`.
    pub way: String,
    /// For a sign-in, who it was granted to.
    pub who: String,
    pub made_at: String,
    pub last_used_at: Option<String>,
    pub cut_off_at: Option<String>,
}

/// A line of an agent's log.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    pub at: String,
    pub tool: Option<String>,
    pub arguments: Option<String>,
    pub refusal: Option<String>,
}

/// A key just made: shown this once.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MadeKey {
    pub id: String,
    pub name: String,
    pub way: String,
    pub key: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct KeyAsked {
    pub name: String,
    /// `mcp` or `api`.
    pub way: String,
}

fn e(err: impl ToString) -> String {
    err.to_string()
}

/// The sign-ins waiting, the ended ones gone first.
pub fn waiting(store: &AgentsBook, now: Timestamp) -> Result<Vec<Waiting>, String> {
    store.end_expired(now - super::REQUEST_LIFE).map_err(e)?;
    let mut out = Vec::new();
    for r in store.waiting().map_err(e)? {
        let client = store.client(&r.client_id).map_err(e)?;
        out.push(Waiting {
            id: r.id,
            code: r.code,
            who: r.who,
            name: client.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
            checked: client.as_ref().is_some_and(|c| c.metadata),
            // registered as https: should one not read as such, it is shown whole
            redirect_host: super::fetch::https_host(&r.redirect_uri).map_or_else(|_| r.redirect_uri.clone(), |h| h.0),
            made_at: r.made_at.to_string(),
        });
    }
    Ok(out)
}

/// Every grant and key, the cut off ones last.
pub fn connected(store: &AgentsBook) -> Result<Vec<Connected>, String> {
    let mut out: Vec<Connected> = store
        .grants()
        .map_err(e)?
        .into_iter()
        .map(|g| Connected { id: g.id, name: g.name, way: "sign-in".into(), who: g.who, made_at: g.made_at.to_string(), last_used_at: g.last_used_at.map(|t| t.to_string()), cut_off_at: g.withdrawn_at.map(|t| t.to_string()) })
        .collect();
    out.extend(store.keys().map_err(e)?.into_iter().map(|k| Connected {
        id: k.id,
        name: k.name,
        way: if k.way == "mcp" { "mcp-key".into() } else { "api-key".into() },
        who: String::new(),
        made_at: k.made_at.to_string(),
        last_used_at: k.last_used_at.map(|t| t.to_string()),
        cut_off_at: k.withdrawn_at.map(|t| t.to_string()),
    }));
    out.sort_by(|a, b| a.cut_off_at.is_some().cmp(&b.cut_off_at.is_some()).then_with(|| a.made_at.cmp(&b.made_at)));
    Ok(out)
}

/// The person's answer to a waiting sign-in. Refused when it is not waiting (it
/// ended, or was answered).
pub fn answer(store: &AgentsBook, id: &str, allowed: bool, now: Timestamp) -> Result<(), String> {
    store.end_expired(now - super::REQUEST_LIFE).map_err(e)?;
    if store.answer(id, allowed, now).map_err(e)? {
        Ok(())
    } else {
        Err("This sign-in is no longer waiting: it ended, or was answered.".into())
    }
}

/// A key made for `way`, named by the person: the key itself, this once.
pub fn make_key(store: &AgentsBook, asked: &KeyAsked, now: Timestamp) -> Result<MadeKey, String> {
    let name: String = asked.name.chars().filter(|c| !c.is_control()).take(60).collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the key a name: the agent it is for.".into());
    }
    if asked.way != "mcp" && asked.way != "api" {
        return Err(format!("{:?} is not a way in", asked.way));
    }
    let (id, key) = (secret::id("bhk_"), secret::secret(if asked.way == "mcp" { "bhm_" } else { "bhj_" }));
    store.add_key(&id, &name, &asked.way, &secret::hash(&key), now).map_err(e)?;
    Ok(MadeKey { id, name, way: asked.way.clone(), key })
}

/// A grant or a key cut off, at once.
pub fn cut_off(store: &AgentsBook, id: &str, now: Timestamp) -> Result<(), String> {
    let done = if id.starts_with("bhg_") { store.withdraw_grant(id, now).map_err(e)? } else { store.withdraw_key(id, now).map_err(e)? };
    if done {
        Ok(())
    } else {
        Err("No agent by that id is connected.".into())
    }
}

/// An agent's log, newest first.
pub fn log(store: &AgentsBook, agent: &str) -> Result<Vec<LogLine>, String> {
    Ok(store.reads(agent, 200).map_err(e)?.into_iter().map(|r| LogLine { at: r.at.to_string(), tool: r.tool, arguments: r.arguments, refusal: r.refusal }).collect())
}
