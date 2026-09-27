-- Each account's monthly statement as the broker issued it
-- (docs/plans/statement-gaps.md): the reply kept whole, as it came, read once
-- and kept, keyed by the broker's own id for the account and the month's first
-- day. What it states and the book's transactions are reconciled from it, and a
-- movement the activity feed left out is booked from it as its own record.
CREATE TABLE monthly_statements (
    connection_id TEXT NOT NULL REFERENCES broker_connections(id),
    account_key TEXT NOT NULL,
    month TEXT NOT NULL,
    payload TEXT NOT NULL,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    PRIMARY KEY (connection_id, account_key, month)
) STRICT;
