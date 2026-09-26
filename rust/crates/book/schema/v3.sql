-- user_version 3
CREATE INDEX account_refs_account ON account_refs (account_id);
CREATE INDEX adjustments_applies ON adjustments (applies_record, applies_leg);
CREATE INDEX declared_distributions_read ON declared_distributions (read_id);
CREATE INDEX declared_reads_instrument ON declared_reads (instrument_id, read_at);
CREATE INDEX fx_reads_currency ON fx_reads (currency, first_day);
CREATE INDEX instrument_refs_instrument ON instrument_refs (instrument_id);
CREATE INDEX instrument_routes_value ON instrument_routes (scheme, value);
CREATE INDEX instrument_sightings_instrument ON instrument_sightings (instrument_id, day);
CREATE INDEX link_records_record ON link_records (record_id);
CREATE INDEX record_problems_record ON record_problems (record_id);
CREATE INDEX record_refs_record ON record_refs (record_id);
CREATE UNIQUE INDEX source_records_key ON source_records (COALESCE(connection_id, ''), source, source_key);
CREATE UNIQUE INDEX trade_groups_legacy_key ON trade_groups (legacy_key) WHERE legacy_key IS NOT NULL;
CREATE UNIQUE INDEX trades_anchor ON trades (anchor_record, anchor_leg, anchor_instrument) WHERE anchor_record IS NOT NULL;
CREATE UNIQUE INDEX trades_legacy_key ON trades (legacy_key) WHERE legacy_key IS NOT NULL;
CREATE INDEX transactions_account ON transactions (account_id, trade_date);
CREATE INDEX transactions_instrument ON transactions (instrument_id);
CREATE TABLE account_refs (
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    PRIMARY KEY (scheme, value)
) STRICT;
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
CREATE TABLE adjustment_legs (
    record_id TEXT NOT NULL,
    leg TEXT NOT NULL,
    position INTEGER NOT NULL,
    from_instrument TEXT REFERENCES instruments(id),
    to_instrument TEXT REFERENCES instruments(id),
    units_per_unit TEXT,
    cost_share TEXT,
    cash_per_unit TEXT,
    cash_currency TEXT,
    cost TEXT,
    cost_currency TEXT,
    acquired TEXT,
    PRIMARY KEY (record_id, leg, position),
    FOREIGN KEY (record_id, leg) REFERENCES adjustments(record_id, leg),
    CHECK ((cash_per_unit IS NULL) = (cash_currency IS NULL)),
    CHECK ((cost IS NULL) = (cost_currency IS NULL)),
    CHECK (from_instrument IS NOT NULL OR to_instrument IS NOT NULL)
) STRICT;
CREATE TABLE adjustments (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    leg TEXT NOT NULL,
    applies_record TEXT NOT NULL,
    applies_leg TEXT NOT NULL,
    PRIMARY KEY (record_id, leg)
) STRICT;
CREATE TABLE bank_holidays (
    day TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    received_at TEXT NOT NULL
) STRICT;
CREATE TABLE broker_connections (
    id TEXT PRIMARY KEY,
    broker TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
CREATE TABLE "declared_distributions" (
    read_id INTEGER NOT NULL REFERENCES declared_reads(id),
    ex_date TEXT NOT NULL,
    record_date TEXT,
    pay_date TEXT,
    amount TEXT NOT NULL,
    reinvested TEXT,
    currency TEXT NOT NULL CHECK (length(currency) = 3)
) STRICT;
CREATE TABLE declared_reads (
    id INTEGER PRIMARY KEY,
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    source TEXT NOT NULL,
    read_at TEXT NOT NULL
) STRICT;
CREATE TABLE fx_rates (
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    day TEXT NOT NULL,
    rate TEXT NOT NULL,
    source TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (currency, day, rate)
) STRICT;
CREATE TABLE fx_reads (
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    first_day TEXT NOT NULL,
    last_day TEXT NOT NULL,
    source TEXT NOT NULL,
    received_at TEXT NOT NULL,
    CHECK (first_day <= last_day)
) STRICT;
CREATE TABLE fx_series (
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    source TEXT NOT NULL,
    first_day TEXT NOT NULL,
    last_day TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (currency, source),
    CHECK (first_day <= last_day)
) STRICT;
CREATE TABLE instrument_refs (
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    PRIMARY KEY (scheme, value)
) STRICT;
CREATE TABLE instrument_routes (
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    PRIMARY KEY (instrument_id, scheme, value)
) STRICT;
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
CREATE TABLE instruments (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    currency TEXT NOT NULL,
    issuer_id TEXT REFERENCES issuers(id),
    created_at TEXT NOT NULL
) STRICT;
CREATE TABLE issuers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
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
CREATE TABLE link_records (
    link_id TEXT NOT NULL REFERENCES links(id),
    record_id TEXT NOT NULL REFERENCES source_records(id),
    side TEXT NOT NULL CHECK (side IN ('from', 'to')),
    PRIMARY KEY (link_id, record_id)
) STRICT;
CREATE TABLE links (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('supersedes')),
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;
CREATE TABLE option_terms (
    instrument_id TEXT PRIMARY KEY REFERENCES instruments(id),
    underlying_id TEXT NOT NULL REFERENCES instruments(id),
    expiry TEXT NOT NULL,
    strike TEXT NOT NULL,
    option_right TEXT NOT NULL CHECK (option_right IN ('call', 'put')),
    multiplier TEXT,
    source TEXT NOT NULL
) STRICT;
CREATE TABLE record_problems (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    code TEXT NOT NULL,
    detail TEXT NOT NULL
) STRICT;
CREATE TABLE record_refs (
    scheme TEXT NOT NULL,
    value TEXT NOT NULL,
    record_id TEXT NOT NULL REFERENCES source_records(id),
    PRIMARY KEY (scheme, value, record_id)
) STRICT;
CREATE TABLE record_revisions (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    revision INTEGER NOT NULL CHECK (revision >= 1),
    received_at TEXT NOT NULL,
    payload TEXT NOT NULL,
    PRIMARY KEY (record_id, revision)
) STRICT;
CREATE TABLE recorded_closes (
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    day TEXT NOT NULL,
    close TEXT NOT NULL,
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    source TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, day)
) STRICT;
CREATE TABLE schema_migrations (
                        number INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        applied_at TEXT NOT NULL,
                        app_version TEXT NOT NULL
                     ) STRICT;
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
CREATE TABLE stated_frequencies (
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    per_year INTEGER NOT NULL CHECK (per_year > 0),
    source TEXT NOT NULL,
    stated_at TEXT,
    received_at TEXT NOT NULL,
    PRIMARY KEY (instrument_id, received_at)
) STRICT;
CREATE TABLE trade_group_members (
    group_id TEXT NOT NULL REFERENCES trade_groups(id),
    position INTEGER NOT NULL,
    trade_id TEXT NOT NULL REFERENCES trades(id),
    PRIMARY KEY (group_id, position),
    UNIQUE (group_id, trade_id)
) STRICT;
CREATE TABLE trade_groups (
    id TEXT PRIMARY KEY,
    locked INTEGER NOT NULL CHECK (locked IN (0, 1)),
    legacy_key TEXT,
    created_at TEXT NOT NULL
) STRICT;
CREATE TABLE trades (
    id TEXT PRIMARY KEY,
    anchor_record TEXT,
    anchor_leg TEXT,
    orphaned_reason TEXT,
    legacy_key TEXT,
    created_at TEXT NOT NULL, anchor_instrument TEXT REFERENCES instruments(id),
    CHECK ((anchor_record IS NULL) = (anchor_leg IS NULL)),
    CHECK ((anchor_record IS NULL) = (orphaned_reason IS NOT NULL))
) STRICT;
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
