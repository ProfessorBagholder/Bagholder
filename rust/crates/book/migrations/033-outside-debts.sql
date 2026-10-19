-- Debts held outside the broker (docs/plans/debts-held-outside-wealthsimple.md):
-- each outside account the broker's net worth feature states, as each read stated
-- it, kept with when it was read; the newest read is what is owed now.
CREATE TABLE outside_debts (
    connection_id TEXT NOT NULL REFERENCES broker_connections(id),
    read_at TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    institution TEXT NOT NULL,
    last4 TEXT,
    balance TEXT NOT NULL,
    currency TEXT NOT NULL,
    as_of TEXT,
    PRIMARY KEY (connection_id, read_at, entity_id)
) STRICT;

-- The person's answer for a payee no rule settled: the outside debt its payments
-- repay, or none (not a debt). Kept by the payee, so later payments follow it.
CREATE TABLE repayment_answers (
    payee_key TEXT PRIMARY KEY NOT NULL,
    debt_id TEXT,
    answered_at TEXT NOT NULL
) STRICT;
