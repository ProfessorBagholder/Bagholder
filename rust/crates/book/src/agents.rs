//! Agents elsewhere (`docs/plans/agents-from-anywhere.md`, §5): who may read
//! through the agents-only address, and what each read. Only these tables are
//! reached through `AgentsBook`, so the agents' listener, given one, can touch
//! nothing else of the book. Every secret is kept as its SHA-256, handed in by the
//! caller; every time is the caller's, and an expiry is checked when the row is
//! next touched, never on a clock.

use rusqlite::{params, OptionalExtension};

use crate::text::{at as at_text, instant};
use crate::{Book, BookError, Result};

/// The book, reached for the agent tables alone.
pub struct AgentsBook {
    book: Book,
}

impl Book {
    /// This connection, for the agent tables alone.
    pub fn agents(self) -> AgentsBook {
        AgentsBook { book: self }
    }
}

/// A client asking to sign in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    /// Its metadata document was fetched (`true`), or it registered itself.
    pub metadata: bool,
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub made_at: jiff::Timestamp,
    pub approved: bool,
}

/// A sign-in waiting on the person, or answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub code: String,
    /// A registered client's id, or a metadata document's https URL: that client is
    /// kept only once the person allows it.
    pub client_id: String,
    /// The name the client gave as it asked.
    pub client_name: String,
    pub who: String,
    pub redirect_uri: String,
    pub state: Option<String>,
    pub challenge: String,
    pub resource: String,
    pub made_at: jiff::Timestamp,
    /// `Some(true)` allowed, `Some(false)` denied, `None` waiting.
    pub answer: Option<bool>,
    pub code_at: Option<jiff::Timestamp>,
    pub code_used: bool,
}

/// What a new request states.
pub struct NewRequest<'a> {
    pub id: &'a str,
    pub code: &'a str,
    pub client_id: &'a str,
    pub client_name: &'a str,
    pub who: &'a str,
    pub redirect_uri: &'a str,
    pub state: Option<&'a str>,
    pub challenge: &'a str,
    pub resource: &'a str,
}

/// A client the person allowed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub id: String,
    pub client_id: String,
    pub name: String,
    pub who: String,
    pub resource: String,
    pub made_at: jiff::Timestamp,
    pub last_used_at: Option<jiff::Timestamp>,
    pub withdrawn_at: Option<jiff::Timestamp>,
}

/// A key the person made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub id: String,
    pub name: String,
    /// `mcp` (`/mcp/key`) or `api` (the JSON API).
    pub way: String,
    pub made_at: jiff::Timestamp,
    pub last_used_at: Option<jiff::Timestamp>,
    pub withdrawn_at: Option<jiff::Timestamp>,
}

/// A token, as its hash finds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub refresh: bool,
    pub grant: Grant,
    pub expires_at: jiff::Timestamp,
    pub used_at: Option<jiff::Timestamp>,
}

/// One line of an agent's log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Read {
    pub agent: String,
    pub at: jiff::Timestamp,
    pub tool: Option<String>,
    pub arguments: Option<String>,
    pub refusal: Option<String>,
}

/// How many lines of each agent's log are kept.
pub const READS_KEPT: i64 = 1_000;

/// Every table this module keeps: what Clear data's Agents kind empties.
pub const TABLES: [&str; 6] = ["agent_reads", "agent_tokens", "agent_keys", "agent_grants", "agent_requests", "agent_clients"];

fn when(table: &'static str, column: &'static str, v: Option<String>) -> Result<Option<jiff::Timestamp>> {
    v.map(|v| instant(table, column, &v)).transpose()
}

fn uris(v: &str) -> Result<Vec<String>> {
    serde_json::from_str(v).map_err(|e| crate::text::corrupt("agent_clients", "redirect_uris", v, e))
}

impl AgentsBook {
    /// Have `heard` run after each commit on this connection: the page's own
    /// status is told a request is waiting, or a grant or key moved.
    pub fn on_commit(&self, heard: std::sync::Arc<dyn Fn() + Send + Sync>) -> rusqlite::Result<()> {
        bagholder_sqlite::on_commit(self.book.conn(), heard)
    }

    fn conn(&self) -> &rusqlite::Connection {
        self.book.conn()
    }

    /// One transaction: all of it or none.
    pub fn atomically<T>(&self, work: impl FnOnce() -> Result<T>) -> Result<T> {
        self.book.atomically(work)
    }

    // ---- clients ----

    /// A client kept: a metadata client's document read again replaces what was
    /// kept of it; a registered client is new. Beyond `unapproved_kept` registered
    /// clients never approved, the oldest go, with their requests.
    pub fn keep_client(&self, c: &Client, unapproved_kept: usize) -> Result<()> {
        self.atomically(|| {
            let kind = if c.metadata { "metadata" } else { "registered" };
            let redirects = serde_json::to_string(&c.redirect_uris).map_err(|e| BookError::Refused(e.to_string()))?;
            self.conn().execute(
                "INSERT INTO agent_clients(id, kind, name, redirect_uris, made_at, approved) VALUES (?1, ?2, ?3, ?4, ?5, 0)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, redirect_uris = excluded.redirect_uris",
                params![c.id, kind, c.name, redirects, at_text(c.made_at)],
            )?;
            if !c.metadata {
                let beyond = "SELECT id FROM agent_clients WHERE kind = 'registered' AND approved = 0
                              ORDER BY made_at DESC, rowid DESC LIMIT -1 OFFSET ?1";
                self.conn().execute(&format!("DELETE FROM agent_requests WHERE client_id IN ({beyond})"), [unapproved_kept as i64])?;
                self.conn().execute(&format!("DELETE FROM agent_clients WHERE id IN ({beyond})"), [unapproved_kept as i64])?;
            }
            Ok(())
        })
    }

    pub fn client(&self, id: &str) -> Result<Option<Client>> {
        let row: Option<(String, String, String, String, String, i64)> = self
            .conn()
            .query_row("SELECT id, kind, name, redirect_uris, made_at, approved FROM agent_clients WHERE id = ?", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .optional()?;
        row.map(|(id, kind, name, redirects, made, approved)| {
            Ok(Client { id, metadata: kind == "metadata", name, redirect_uris: uris(&redirects)?, made_at: instant("agent_clients", "made_at", &made)?, approved: approved == 1 })
        })
        .transpose()
    }

    /// How many registered clients were never approved.
    pub fn unapproved(&self) -> Result<usize> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM agent_clients WHERE kind = 'registered' AND approved = 0", [], |r| r.get::<_, i64>(0))? as usize)
    }

    // ---- requests ----

    /// Every request that has ended, gone: one made before `older_than` with no
    /// code handed (never answered, denied and never read, or allowed and never
    /// collected), and one whose code was handed before `codes_older_than` and never
    /// exchanged (an exchanged one goes at the exchange). With them, each
    /// registered client never approved that has no request left.
    pub fn end_expired(&self, older_than: jiff::Timestamp, codes_older_than: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            self.conn().execute("DELETE FROM agent_requests WHERE (code_at IS NULL AND made_at < ?1) OR code_at < ?2", params![at_text(older_than), at_text(codes_older_than)])?;
            self.conn().execute(
                "DELETE FROM agent_clients WHERE kind = 'registered' AND approved = 0 AND made_at < ?1
                 AND id NOT IN (SELECT client_id FROM agent_requests)",
                [at_text(older_than)],
            )?;
            Ok(())
        })
    }

    /// The requests made at or after `made_since` still waiting on the person,
    /// oldest first: an older one has ended, whether or not it is gone yet.
    pub fn waiting(&self, made_since: jiff::Timestamp) -> Result<Vec<Request>> {
        self.requests_where("answer IS NULL AND made_at >= ?1", [at_text(made_since)])
    }

    pub fn request(&self, id: &str) -> Result<Option<Request>> {
        Ok(self.requests_where("id = ?1", [id])?.pop())
    }

    /// The request whose authorization code hashes to `hash`.
    pub fn request_by_code(&self, hash: &[u8]) -> Result<Option<Request>> {
        Ok(self.requests_where("code_hash = ?1", [hash])?.pop())
    }

    fn requests_where<P: rusqlite::Params>(&self, filter: &str, p: P) -> Result<Vec<Request>> {
        let sql = format!(
            "SELECT id, code, client_id, client_name, who, redirect_uri, state, challenge, resource, made_at, answer, code_at, code_used
             FROM agent_requests WHERE {filter} ORDER BY made_at, rowid"
        );
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(p, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<String>>(11)?,
                r.get::<_, i64>(12)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, code, client_id, client_name, who, redirect_uri, state, challenge, resource, made, answer, code_at, used) = row?;
            out.push(Request {
                id,
                code,
                client_id,
                client_name,
                who,
                redirect_uri,
                state,
                challenge,
                resource,
                made_at: instant("agent_requests", "made_at", &made)?,
                answer: answer.map(|a| a == "allowed"),
                code_at: when("agent_requests", "code_at", code_at)?,
                code_used: used == 1,
            });
        }
        Ok(out)
    }

    /// A new request, unless `pending_kept` are already waiting: then `false`.
    pub fn add_request(&self, r: &NewRequest, at: jiff::Timestamp, pending_kept: usize) -> Result<bool> {
        self.atomically(|| {
            let waiting: i64 = self.conn().query_row("SELECT COUNT(*) FROM agent_requests WHERE answer IS NULL", [], |x| x.get(0))?;
            if waiting as usize >= pending_kept {
                return Ok(false);
            }
            self.conn().execute(
                "INSERT INTO agent_requests(id, code, client_id, client_name, who, redirect_uri, state, challenge, resource, made_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![r.id, r.code, r.client_id, r.client_name, r.who, r.redirect_uri, r.state, r.challenge, r.resource, at_text(at)],
            )?;
            Ok(true)
        })
    }

    /// The person's answer to a request still waiting: false when it is not.
    pub fn answer(&self, id: &str, allowed: bool, at: jiff::Timestamp) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE agent_requests SET answer = ?2, answered_at = ?3 WHERE id = ?1 AND answer IS NULL",
            params![id, if allowed { "allowed" } else { "denied" }, at_text(at)],
        )?;
        Ok(n == 1)
    }

    /// The authorization code handed out for an allowed request, by its hash, once:
    /// false when one was handed already.
    pub fn hand_code(&self, id: &str, hash: &[u8], at: jiff::Timestamp) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE agent_requests SET code_hash = ?2, code_at = ?3 WHERE id = ?1 AND answer = 'allowed' AND code_hash IS NULL",
            params![id, hash, at_text(at)],
        )?;
        Ok(n == 1)
    }

    /// A code exchanged: true the first time, false ever after.
    pub fn use_code(&self, id: &str) -> Result<bool> {
        Ok(self.conn().execute("UPDATE agent_requests SET code_used = 1 WHERE id = ?1 AND code_used = 0", [id])? == 1)
    }

    /// A request ended (exchanged, or refused for good).
    pub fn drop_request(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM agent_requests WHERE id = ?", [id])?;
        Ok(())
    }

    // ---- grants and tokens ----

    /// A grant for `client`, made when its request's code is exchanged: the client
    /// is approved from then on.
    pub fn add_grant(&self, id: &str, client: &str, name: &str, who: &str, resource: &str, at: jiff::Timestamp) -> Result<()> {
        self.atomically(|| {
            self.conn().execute(
                "INSERT INTO agent_grants(id, client_id, name, who, resource, made_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, client, name, who, resource, at_text(at)],
            )?;
            self.conn().execute("UPDATE agent_clients SET approved = 1 WHERE id = ?", [client])?;
            Ok(())
        })
    }

    pub fn grants(&self) -> Result<Vec<Grant>> {
        self.grants_where("1 = 1", [])
    }

    pub fn grant(&self, id: &str) -> Result<Option<Grant>> {
        Ok(self.grants_where("id = ?1", [id])?.pop())
    }

    fn grants_where<P: rusqlite::Params>(&self, filter: &str, p: P) -> Result<Vec<Grant>> {
        let sql = format!("SELECT id, client_id, name, who, resource, made_at, last_used_at, withdrawn_at FROM agent_grants WHERE {filter} ORDER BY made_at, rowid");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(p, |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, Option<String>>(7)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, client_id, name, who, resource, made, used, withdrawn) = row?;
            out.push(Grant {
                id,
                client_id,
                name,
                who,
                resource,
                made_at: instant("agent_grants", "made_at", &made)?,
                last_used_at: when("agent_grants", "last_used_at", used)?,
                withdrawn_at: when("agent_grants", "withdrawn_at", withdrawn)?,
            });
        }
        Ok(out)
    }

    /// A grant cut off: every token of it is refused from now on.
    pub fn withdraw_grant(&self, id: &str, at: jiff::Timestamp) -> Result<bool> {
        self.atomically(|| {
            let n = self.conn().execute("UPDATE agent_grants SET withdrawn_at = ?2 WHERE id = ?1 AND withdrawn_at IS NULL", params![id, at_text(at)])?;
            self.conn().execute("DELETE FROM agent_tokens WHERE grant_id = ?", [id])?;
            Ok(n == 1)
        })
    }

    pub fn add_token(&self, hash: &[u8], refresh: bool, grant: &str, expires_at: jiff::Timestamp) -> Result<()> {
        self.conn().execute(
            "INSERT INTO agent_tokens(hash, kind, grant_id, expires_at) VALUES (?1, ?2, ?3, ?4)",
            params![hash, if refresh { "refresh" } else { "access" }, grant, at_text(expires_at)],
        )?;
        Ok(())
    }

    /// The token hashing to `hash`, with its grant.
    pub fn token(&self, hash: &[u8]) -> Result<Option<Token>> {
        let row: Option<(String, String, String, Option<String>)> = self
            .conn()
            .query_row("SELECT kind, grant_id, expires_at, used_at FROM agent_tokens WHERE hash = ?", [hash], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .optional()?;
        let Some((kind, grant, expires, used)) = row else { return Ok(None) };
        let Some(grant) = self.grant(&grant)? else { return Ok(None) };
        Ok(Some(Token { refresh: kind == "refresh", grant, expires_at: instant("agent_tokens", "expires_at", &expires)?, used_at: when("agent_tokens", "used_at", used)? }))
    }

    /// A refresh token used: true the first time, false when it was used before.
    pub fn use_refresh(&self, hash: &[u8], at: jiff::Timestamp) -> Result<bool> {
        Ok(self.conn().execute("UPDATE agent_tokens SET used_at = ?2 WHERE hash = ?1 AND kind = 'refresh' AND used_at IS NULL", params![hash, at_text(at)])? == 1)
    }

    // ---- keys ----

    pub fn add_key(&self, id: &str, name: &str, way: &str, hash: &[u8], at: jiff::Timestamp) -> Result<()> {
        self.conn().execute("INSERT INTO agent_keys(id, name, way, hash, made_at) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, name, way, hash, at_text(at)])?;
        Ok(())
    }

    pub fn keys(&self) -> Result<Vec<Key>> {
        self.keys_where("1 = 1", [])
    }

    /// The key hashing to `hash`.
    pub fn key_by_hash(&self, hash: &[u8]) -> Result<Option<Key>> {
        Ok(self.keys_where("hash = ?1", [hash])?.pop())
    }

    fn keys_where<P: rusqlite::Params>(&self, filter: &str, p: P) -> Result<Vec<Key>> {
        let sql = format!("SELECT id, name, way, made_at, last_used_at, withdrawn_at FROM agent_keys WHERE {filter} ORDER BY made_at, rowid");
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(p, |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, way, made, used, withdrawn) = row?;
            out.push(Key { id, name, way, made_at: instant("agent_keys", "made_at", &made)?, last_used_at: when("agent_keys", "last_used_at", used)?, withdrawn_at: when("agent_keys", "withdrawn_at", withdrawn)? });
        }
        Ok(out)
    }

    pub fn withdraw_key(&self, id: &str, at: jiff::Timestamp) -> Result<bool> {
        Ok(self.conn().execute("UPDATE agent_keys SET withdrawn_at = ?2 WHERE id = ?1 AND withdrawn_at IS NULL", params![id, at_text(at)])? == 1)
    }

    // ---- the log ----

    /// A read or a refusal logged, the agent's last use with a read, and the
    /// agent's log trimmed to the last `READS_KEPT`.
    pub fn log(&self, r: &Read) -> Result<()> {
        self.atomically(|| {
            self.conn().execute(
                "INSERT INTO agent_reads(agent, at, tool, arguments, refusal) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![r.agent, at_text(r.at), r.tool, r.arguments, r.refusal],
            )?;
            if r.tool.is_some() {
                self.conn().execute("UPDATE agent_grants SET last_used_at = ?2 WHERE id = ?1", params![r.agent, at_text(r.at)])?;
                self.conn().execute("UPDATE agent_keys SET last_used_at = ?2 WHERE id = ?1", params![r.agent, at_text(r.at)])?;
            }
            self.conn().execute(
                "DELETE FROM agent_reads WHERE agent = ?1 AND seq NOT IN (SELECT seq FROM agent_reads WHERE agent = ?1 ORDER BY seq DESC LIMIT ?2)",
                params![r.agent, READS_KEPT],
            )?;
            Ok(())
        })
    }

    /// An agent's log, newest first, at most `limit` lines.
    pub fn reads(&self, agent: &str, limit: usize) -> Result<Vec<Read>> {
        let mut stmt = self.conn().prepare("SELECT agent, at, tool, arguments, refusal FROM agent_reads WHERE agent = ?1 ORDER BY seq DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![agent, limit as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<String>>(4)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (agent, at, tool, arguments, refusal) = row?;
            out.push(Read { agent, at: instant("agent_reads", "at", &at)?, tool, arguments, refusal });
        }
        Ok(out)
    }

    // ---- settings ----

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.book.setting(key)
    }

    pub fn set_setting(&self, key: &str, value: Option<&str>, at: jiff::Timestamp) -> Result<()> {
        self.book.set_setting(key, value, at)
    }
}
