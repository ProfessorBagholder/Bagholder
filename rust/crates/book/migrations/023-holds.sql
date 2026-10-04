-- What a broker holds against an account for a row not yet final
-- (docs/plans/broker-check-reserved-cash.md): derived from the record by its
-- mapping like its transactions, replaced with each derivation; a kind's amount
-- NULL is unstated.
CREATE TABLE record_holds (
    record_id TEXT PRIMARY KEY REFERENCES source_records(id),
    account_id TEXT NOT NULL REFERENCES accounts(id),
    kind TEXT NOT NULL CHECK (kind IN ('buy', 'put-sale', 'withdrawal')),
    currency TEXT,
    instrument_id TEXT REFERENCES instruments(id),
    amount TEXT,
    quantity TEXT,
    premium TEXT
) STRICT;

-- The holds as they stood when the broker stated an account's cash, one row a
-- hold, kept with that statement as the broker's balance was stated beside it.
CREATE TABLE statement_holds (
    statement_id TEXT NOT NULL REFERENCES statements(id),
    record_id TEXT NOT NULL REFERENCES source_records(id),
    kind TEXT NOT NULL CHECK (kind IN ('buy', 'put-sale', 'withdrawal')),
    currency TEXT,
    instrument_id TEXT REFERENCES instruments(id),
    amount TEXT,
    quantity TEXT,
    premium TEXT,
    PRIMARY KEY (statement_id, record_id)
) STRICT;
