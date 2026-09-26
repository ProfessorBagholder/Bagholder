-- What a broker states beside its rows (docs/plans/stage-3b-wealthsimple.md,
-- "The book's new tables"): the reads they came from, the links between
-- accounts, each account's value per day, its cash and units at a moment, when
-- its activity was last read in full, and the two sides of a move of holdings
-- between the person's accounts. Each is kept as the broker stated it.

-- each read of a broker, for what it stated
CREATE TABLE broker_reads (
    id TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL REFERENCES broker_connections(id),
    part TEXT NOT NULL,
    at TEXT NOT NULL
) STRICT;

-- an account and the account the broker states it is linked to
CREATE TABLE account_links (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    linked_to TEXT NOT NULL REFERENCES accounts(id),
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    PRIMARY KEY (account_id, linked_to)
) STRICT;

-- an account's value and net deposits on a day, in the currency stated; a day
-- the broker states again differently is a new row beside the first, and the
-- newest read is the one used
CREATE TABLE account_days (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    day TEXT NOT NULL,
    net_value TEXT NOT NULL,
    net_deposits TEXT NOT NULL,
    currency TEXT NOT NULL,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    PRIMARY KEY (account_id, day, read_id)
) STRICT;

-- an account's cash per currency at a moment, or its units per instrument as
-- of a day, as the broker stated them
CREATE TABLE statements (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    kind TEXT NOT NULL CHECK (kind IN ('cash', 'units')),
    -- the moment the cash was stated (when it was asked), for cash
    stated_at TEXT,
    -- the day the units are stated as of, for units
    as_of_day TEXT,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    CHECK ((kind = 'cash') = (stated_at IS NOT NULL)),
    CHECK ((kind = 'units') = (as_of_day IS NOT NULL))
) STRICT;
CREATE TABLE statement_cash (
    statement_id TEXT NOT NULL REFERENCES statements(id),
    currency TEXT NOT NULL,
    amount TEXT NOT NULL,
    PRIMARY KEY (statement_id, currency)
) STRICT;
CREATE TABLE statement_units (
    statement_id TEXT NOT NULL REFERENCES statements(id),
    instrument_id TEXT NOT NULL REFERENCES instruments(id),
    quantity TEXT NOT NULL,
    -- the broker's own book value for the position, kept as its statement,
    -- never taken as a cost (brief 07 §5)
    book_value TEXT,
    book_value_currency TEXT,
    PRIMARY KEY (statement_id, instrument_id),
    CHECK ((book_value IS NULL) = (book_value_currency IS NULL))
) STRICT;

-- each read of an account's activity, and whether every page of it was read
CREATE TABLE activity_reads (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    read_at TEXT NOT NULL,
    complete INTEGER NOT NULL CHECK (complete IN (0, 1)),
    PRIMARY KEY (account_id, read_at)
) STRICT;

-- the two sides of a move of holdings between the person's accounts, joined by
-- what the broker's rows state (their accounts and instant), never by amounts
CREATE TABLE transfer_links (
    out_record TEXT NOT NULL REFERENCES source_records(id),
    out_leg TEXT NOT NULL,
    in_record TEXT NOT NULL REFERENCES source_records(id),
    in_leg TEXT NOT NULL,
    PRIMARY KEY (out_record, out_leg)
) STRICT;
