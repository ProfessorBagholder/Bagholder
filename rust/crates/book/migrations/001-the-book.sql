-- The book, schema 1 (docs/architecture.md §5, §6; docs/plans/stage-1-foundation.md).
-- Ids are UUID v7 text; decimals are canonical decimal text (bagholder_core::Dec);
-- instants are RFC 3339 UTC text; days are YYYY-MM-DD. Every table is STRICT.

-- one login at one broker
CREATE TABLE broker_connections (
    id TEXT PRIMARY KEY,
    broker TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

-- an account in Bagholder's vocabulary; a broker type Bagholder does not know is
-- kept in the broker's words instead of a kind
CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL REFERENCES broker_connections(id),
    kind TEXT,
    registration TEXT,
    managed INTEGER CHECK (managed IN (0, 1)),
    joint INTEGER CHECK (joint IN (0, 1)),
    unrecognised_type TEXT,
    status TEXT NOT NULL CHECK (status IN ('open', 'closed')),
    nickname TEXT,
    created_at TEXT NOT NULL,
    CHECK ((kind IS NULL) = (unrecognised_type IS NOT NULL)),
    CHECK ((kind IS NULL) = (registration IS NULL)),
    CHECK ((kind IS NULL) = (managed IS NULL)),
    CHECK ((kind IS NULL) = (joint IS NULL))
) STRICT;

-- a broker's own ids for an account: several ids (a currency each) can be one account
CREATE TABLE account_refs (
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    PRIMARY KEY (scheme, value)
) STRICT;
CREATE INDEX account_refs_account ON account_refs (account_id);

CREATE TABLE issuers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE instruments (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    currency TEXT NOT NULL,
    issuer_id TEXT REFERENCES issuers(id),
    created_at TEXT NOT NULL
) STRICT;

-- the identifiers that say which instrument a thing is (a broker's security id,
-- ISIN, CUSIP, FIGI, the OCC symbol, a connection's own symbol): one instrument per value
CREATE TABLE instrument_refs (
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    PRIMARY KEY (scheme, value)
) STRICT;
CREATE INDEX instrument_refs_instrument ON instrument_refs (instrument_id);

-- how to ask a source for an instrument (a Yahoo symbol, a TMX form, a CIK, a
-- SEDAR+ profile): never an identity, so one value may route to several instruments
CREATE TABLE instrument_routes (
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (instrument_id, scheme, value)
) STRICT;
CREATE INDEX instrument_routes_value ON instrument_routes (scheme, value);

-- what a record's leg called an instrument, on the record's day: an instrument's
-- names over time are read from the sightings of live records, so a revised or
-- removed record takes its sighting with it
CREATE TABLE instrument_sightings (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    leg TEXT NOT NULL,
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    symbol TEXT NOT NULL,
    venue_mic TEXT,
    venue_name TEXT,
    name TEXT,
    day TEXT NOT NULL,
    PRIMARY KEY (record_id, leg, instrument_id)
) STRICT;
CREATE INDEX instrument_sightings_instrument ON instrument_sightings (instrument_id, day);

-- an option contract's terms; the multiplier is empty until a source states it
CREATE TABLE option_terms (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments(id),
    underlying_id TEXT NOT NULL REFERENCES instruments(id),
    expiry TEXT NOT NULL,
    strike TEXT NOT NULL,
    option_right TEXT NOT NULL CHECK (option_right IN ('call', 'put')),
    multiplier TEXT,
    source TEXT NOT NULL
) STRICT;

-- what a source reported; its content is in its revisions
CREATE TABLE source_records (
    id TEXT PRIMARY KEY,
    connection_id TEXT REFERENCES broker_connections(id),
    source TEXT NOT NULL,
    source_key TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('live', 'superseded', 'removed')),
    -- the version of the source's mapping its transactions and problems came from
    derived_version INTEGER NOT NULL,
    first_received_at TEXT NOT NULL,
    -- when its state last changed: its arrival, or its replacement or removal
    state_changed_at TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX source_records_key ON source_records (COALESCE(connection_id, ''), source, source_key);

-- each different payload a record has had, as canonical JSON
CREATE TABLE record_revisions (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    received_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (record_id, revision)
) STRICT;

-- other ids a record is known by, so a later source can find it
CREATE TABLE record_refs (
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    record_id TEXT NOT NULL REFERENCES source_records(id),
    PRIMARY KEY (scheme, value, record_id)
) STRICT;
CREATE INDEX record_refs_record ON record_refs (record_id);

-- what the person should see about a record
CREATE TABLE record_problems (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    code TEXT NOT NULL,
    detail TEXT NOT NULL
) STRICT;
CREATE INDEX record_problems_record ON record_problems (record_id);

-- Bagholder's transactions, derived from live records; a leg is named by the mapping
CREATE TABLE transactions (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    leg TEXT NOT NULL,
    mapping_version INTEGER NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    occurred_at TEXT,
    trade_date TEXT NOT NULL,
    settle_date TEXT,
    kind TEXT NOT NULL,
    effect TEXT CHECK (effect IN ('open', 'close')),
    instrument_id TEXT REFERENCES instruments(id),
    quantity TEXT,
    price TEXT,
    price_currency TEXT,
    cash TEXT,
    cash_currency TEXT,
    fee TEXT,
    fee_currency TEXT,
    fx_rate TEXT,
    PRIMARY KEY (record_id, leg),
    CHECK ((price IS NULL) = (price_currency IS NULL)),
    CHECK ((cash IS NULL) = (cash_currency IS NULL)),
    CHECK ((fee IS NULL) = (fee_currency IS NULL)),
    CHECK (quantity IS NULL OR instrument_id IS NOT NULL),
    CHECK (price IS NULL OR quantity IS NOT NULL)
) STRICT;
CREATE INDEX transactions_account ON transactions (account_id, trade_date);
CREATE INDEX transactions_instrument ON transactions (instrument_id);

-- one record superseding others, many to many
CREATE TABLE links (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('supersedes')),
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
CREATE TABLE link_records (
    link_id TEXT NOT NULL REFERENCES links(id),
    record_id TEXT NOT NULL REFERENCES source_records(id),
    side TEXT NOT NULL CHECK (side IN ('from', 'to')),
    PRIMARY KEY (link_id, record_id)
) STRICT;
CREATE INDEX link_records_record ON link_records (record_id);

-- a round trip, anchored on its opening transaction; orphaned, with the reason,
-- when that transaction is gone
CREATE TABLE trades (
    id TEXT PRIMARY KEY,
    anchor_record TEXT,
    anchor_leg TEXT,
    orphaned_reason TEXT,
    legacy_key TEXT,
    created_at TEXT NOT NULL,
    CHECK ((anchor_record IS NULL) = (anchor_leg IS NULL)),
    CHECK ((anchor_record IS NULL) = (orphaned_reason IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX trades_anchor ON trades (anchor_record, anchor_leg) WHERE anchor_record IS NOT NULL;
CREATE UNIQUE INDEX trades_legacy_key ON trades (legacy_key) WHERE legacy_key IS NOT NULL;

CREATE TABLE trade_groups (
    id TEXT PRIMARY KEY,
    locked INTEGER NOT NULL CHECK (locked IN (0, 1)),
    legacy_key TEXT,
    created_at TEXT NOT NULL
) STRICT;
CREATE UNIQUE INDEX trade_groups_legacy_key ON trade_groups (legacy_key) WHERE legacy_key IS NOT NULL;
CREATE TABLE trade_group_members (
    group_id TEXT NOT NULL REFERENCES trade_groups(id),
    position INTEGER NOT NULL,
    trade_id TEXT NOT NULL REFERENCES trades(id),
    PRIMARY KEY (group_id, position),
    UNIQUE (group_id, trade_id)
) STRICT;

-- what the person wrote on a trade, or on a group of trades
CREATE TABLE journal (
    id INTEGER PRIMARY KEY,
    trade_id TEXT UNIQUE REFERENCES trades(id),
    group_id TEXT UNIQUE REFERENCES trade_groups(id),
    thesis TEXT NOT NULL,
    grade TEXT CHECK (grade IN ('A', 'B', 'C', 'F')),
    updated_at TEXT NOT NULL,
    CHECK ((trade_id IS NULL) <> (group_id IS NULL))
) STRICT;
CREATE TABLE journal_tags (
    journal_id INTEGER NOT NULL REFERENCES journal(id),
    position INTEGER NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (journal_id, position)
) STRICT;
