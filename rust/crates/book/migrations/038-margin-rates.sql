-- Each margin account's yearly interest rate on what it borrows, by currency, as
-- the broker states it (Wealthsimple's `FetchAccountAppliedRates`, rate type
-- MARGIN_INTEREST), kept as periods like the other statements (migration 26): a
-- fraction, 0.0445 for 4.45%, none for a currency it states no rate in.
CREATE TABLE margin_rates (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    -- the first moment it was stated
    stated_at TEXT NOT NULL,
    currency TEXT NOT NULL,
    rate TEXT NOT NULL,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    first_ns INTEGER NOT NULL,
    first_seq INTEGER NOT NULL,
    last_ns INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    last_read_id TEXT NOT NULL REFERENCES broker_reads(id),
    CHECK (first_ns < last_ns OR (first_ns = last_ns AND first_seq <= last_seq))
) STRICT;
CREATE INDEX margin_rates_account ON margin_rates (account_id, currency, first_ns, first_seq);
