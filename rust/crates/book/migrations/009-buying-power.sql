-- What a margin account can borrow, as the broker states it at a moment
-- (docs/plans/stage-3c-switch.md, §3: balances and buying power): an amount in
-- the currency asked, or the broker's reason it cannot say. Kept beside the
-- account's cash, as the broker stated it.
CREATE TABLE buying_power (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    stated_at TEXT NOT NULL,
    amount TEXT,
    currency TEXT,
    unavailable TEXT,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    PRIMARY KEY (account_id, stated_at, read_id),
    CHECK ((amount IS NULL) = (currency IS NULL)),
    CHECK ((amount IS NULL) <> (unavailable IS NULL))
) STRICT;
