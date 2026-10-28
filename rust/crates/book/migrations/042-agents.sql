-- Agents elsewhere, through the agents-only address (docs/plans/agents-from-anywhere.md,
-- §5): who may read, and what each read. Every secret is kept as the SHA-256 of 32
-- random bytes, never the secret itself; every time is checked when it is next
-- touched, never on a clock.

-- A client signing in: one whose metadata document Bagholder fetched (its id is that
-- document's https URL), kept once the person allows it, or one registered by the
-- agent itself (its id made here). A registered one never approved is dropped with
-- its request, and the oldest beyond the bound when another registers.
CREATE TABLE agent_clients (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('metadata', 'registered')),
    -- the name the agent gave, shown as the agent's own word
    name TEXT NOT NULL,
    -- the redirect URIs, each matched exactly: a JSON array of strings
    redirect_uris TEXT NOT NULL,
    made_at TEXT NOT NULL,
    approved INTEGER NOT NULL DEFAULT 0 CHECK (approved IN (0, 1))
) STRICT;

-- A sign-in waiting on the person's own computer, until allowed, denied or ten
-- minutes old. Its id is 128 random bits, asked by the public page alone; its code
-- is the six letters shown on both pages, never accepted in place of the id.
CREATE TABLE agent_requests (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    -- a registered client's id, or a metadata document's https URL, whose client is
    -- kept only once the person allows it: until then the request holds what it said
    client_id TEXT NOT NULL,
    -- the name the client gave as it asked
    client_name TEXT NOT NULL,
    -- who is asking, as both pages show it: the metadata document's host, or a
    -- registered client's redirect host
    who TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    -- the agent's own state, sent back with the code
    state TEXT,
    -- the PKCE challenge, S256
    challenge TEXT NOT NULL,
    resource TEXT NOT NULL,
    made_at TEXT NOT NULL,
    answer TEXT CHECK (answer IN ('allowed', 'denied')),
    answered_at TEXT,
    -- the authorization code handed to the public page once allowed, as its hash,
    -- and when it was handed; a code is exchanged once
    code_hash BLOB,
    code_at TEXT,
    code_used INTEGER NOT NULL DEFAULT 0 CHECK (code_used IN (0, 1))
) STRICT;

-- A client the person allowed: one grant, until it is cut off.
CREATE TABLE agent_grants (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES agent_clients(id),
    name TEXT NOT NULL,
    who TEXT NOT NULL,
    resource TEXT NOT NULL,
    made_at TEXT NOT NULL,
    last_used_at TEXT,
    withdrawn_at TEXT
) STRICT;

-- The tokens of a grant: an access token for an hour, a refresh token until it is
-- used once (a second use ends the grant).
CREATE TABLE agent_tokens (
    hash BLOB PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('access', 'refresh')),
    grant_id TEXT NOT NULL REFERENCES agent_grants(id) ON DELETE CASCADE,
    expires_at TEXT NOT NULL,
    used_at TEXT
) STRICT;
CREATE INDEX agent_tokens_grant ON agent_tokens (grant_id);

-- A key the person made on their own page, for the key's way in: MCP at /mcp/key
-- or the JSON API.
CREATE TABLE agent_keys (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    way TEXT NOT NULL CHECK (way IN ('mcp', 'api')),
    hash BLOB NOT NULL UNIQUE,
    made_at TEXT NOT NULL,
    last_used_at TEXT,
    withdrawn_at TEXT
) STRICT;

-- Each read an agent made (the tool and its arguments, never the answer), and each
-- refusal; the last thousand of each agent kept, trimmed as one is written.
CREATE TABLE agent_reads (
    seq INTEGER PRIMARY KEY,
    -- the grant's or the key's id; empty for a caller no grant or key names
    agent TEXT NOT NULL,
    at TEXT NOT NULL,
    tool TEXT,
    arguments TEXT,
    refusal TEXT,
    CHECK ((tool IS NULL) <> (refusal IS NULL))
) STRICT;
CREATE INDEX agent_reads_agent ON agent_reads (agent, seq);
