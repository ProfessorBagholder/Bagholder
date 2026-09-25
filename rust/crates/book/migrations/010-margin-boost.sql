-- A cash account Wealthsimple lets back a margin account as collateral (its
-- Margin Boost), as the broker states it with the accounts: each account backs at
-- most one, and a read that no longer states it takes the link away.
CREATE TABLE margin_backing (
    account_id TEXT PRIMARY KEY REFERENCES accounts(id),
    margin_account_id TEXT NOT NULL REFERENCES accounts(id),
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    CHECK (account_id <> margin_account_id)
) STRICT;
