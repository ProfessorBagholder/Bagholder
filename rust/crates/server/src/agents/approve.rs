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
    /// The server it was made for (`https://{host}`). Empty for a key made before
    /// keys named theirs and not used since.
    pub at: String,
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
pub struct Renamed {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct KeyAsked {
    pub name: String,
    /// `mcp` or `api`.
    pub way: String,
    /// The server it is for (`https://{host}`): it is taken only there.
    pub at: String,
}

fn e(err: impl ToString) -> String {
    err.to_string()
}

/// The sign-ins waiting: a read, never a write (the page's status asks it on every
/// step), so an ended one is left out here and goes on the next write.
pub fn waiting(store: &AgentsBook, now: Timestamp) -> Result<Vec<Waiting>, String> {
    let mut out = Vec::new();
    for r in store.waiting(now - super::REQUEST_LIFE).map_err(e)? {
        out.push(Waiting {
            checked: metadata(&r.client_id),
            id: r.id,
            code: r.code,
            who: r.who,
            name: r.client_name,
            made_at: r.made_at.to_string(),
        });
    }
    Ok(out)
}

/// Every grant and key still standing: a removed one has left the list (brief 81
/// §3), its access withdrawn the moment it was removed.
pub fn connected(store: &AgentsBook) -> Result<Vec<Connected>, String> {
    let mut out: Vec<Connected> = store
        .grants()
        .map_err(e)?
        .into_iter()
        .map(|g| Connected { id: g.id, name: g.name, way: "sign-in".into(), who: g.who, at: g.resource.trim_end_matches("/mcp").to_string(), made_at: g.made_at.to_string(), last_used_at: g.last_used_at.map(|t| t.to_string()), cut_off_at: g.withdrawn_at.map(|t| t.to_string()) })
        .collect();
    out.extend(store.keys().map_err(e)?.into_iter().map(|k| Connected {
        id: k.id,
        name: k.name,
        way: if k.way == "mcp" { "mcp-key".into() } else { "api-key".into() },
        who: String::new(),
        at: k.at.unwrap_or_default(),
        made_at: k.made_at.to_string(),
        last_used_at: k.last_used_at.map(|t| t.to_string()),
        cut_off_at: k.withdrawn_at.map(|t| t.to_string()),
    }));
    out.retain(|c| c.cut_off_at.is_none());
    out.sort_by(|a, b| a.made_at.cmp(&b.made_at));
    Ok(out)
}

/// Whether a client id is a metadata document's URL: its client is the document's,
/// fetched as it asks, and kept only once the person allows it.
pub fn metadata(client_id: &str) -> bool {
    client_id.starts_with("https://")
}

/// The person's answer to a waiting sign-in. Refused when it is not waiting (it
/// ended, or was answered). Allowed, a metadata document's client is kept from
/// what its sign-in holds, the first time it is: until now it lived in the sign-in.
pub fn answer(store: &AgentsBook, id: &str, allowed: bool, now: Timestamp) -> Result<(), String> {
    super::end_expired(store, now).map_err(e)?;
    let answered = store.atomically(|| {
        let r = store.request(id)?;
        if !store.answer(id, allowed, now)? {
            return Ok(false);
        }
        if let Some(r) = r.filter(|r| allowed && metadata(&r.client_id)) {
            if store.client(&r.client_id)?.is_none() {
                let c = bagholder_book::agents::Client { id: r.client_id, metadata: true, name: r.client_name, redirect_uris: vec![r.redirect_uri], made_at: now, approved: false };
                store.keep_client(&c, super::UNAPPROVED_KEPT)?;
            }
        }
        Ok(true)
    });
    match answered.map_err(e)? {
        true => Ok(()),
        false => Err("This sign-in is no longer waiting: it ended, or was answered.".into()),
    }
}

/// A key made for `way` at one of `servers`, named by the person: the key itself,
/// this once.
pub fn make_key(store: &AgentsBook, asked: &KeyAsked, servers: &[String], now: Timestamp) -> Result<MadeKey, String> {
    let name: String = asked.name.chars().filter(|c| !c.is_control()).take(60).collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the key a name: the agent it is for.".into());
    }
    if asked.way != "mcp" && asked.way != "api" {
        return Err(format!("{:?} is not a way in", asked.way));
    }
    if !servers.contains(&asked.at) {
        return Err(format!("{} is not one of Bagholder's servers.", asked.at));
    }
    let (id, key) = (secret::id("bhk_"), secret::secret(if asked.way == "mcp" { "bhm_" } else { "bhj_" }));
    store.add_key(&id, &name, &asked.way, &secret::hash(&key), &asked.at, now).map_err(e)?;
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

/// A connected agent named by the person, as they know it: an agent's own name is
/// what it gave (Grok Bot signs in as Cursor), never more than its sender says.
pub fn rename(store: &AgentsBook, asked: &Renamed) -> Result<(), String> {
    let name: String = asked.name.chars().filter(|c| !c.is_control()).take(60).collect();
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the agent a name.".into());
    }
    if store.rename_agent(&asked.id, name).map_err(e)? {
        Ok(())
    } else {
        Err("No agent by that id is connected.".into())
    }
}

/// An agent's log, newest first.
pub fn log(store: &AgentsBook, agent: &str) -> Result<Vec<LogLine>, String> {
    Ok(store.reads(agent, 200).map_err(e)?.into_iter().map(|r| LogLine { at: r.at.to_string(), tool: r.tool, arguments: r.arguments, refusal: r.refusal }).collect())
}
