-- A bracket holds the person's sale from the ticket until it fills or ends
-- (docs/plans/stage-money.md, part A): the phase `selling` joins the phases a
-- bracket's fold can stand in. SQLite changes a CHECK only by building the table
-- again; its orders and log keep pointing at it by id, checked at the commit.
PRAGMA defer_foreign_keys = ON;
CREATE TABLE brackets_v25 (
    id TEXT PRIMARY KEY,
    -- where its exits trade: the broker's own ids, as they are sent
    broker TEXT NOT NULL,
    broker_account TEXT NOT NULL,
    broker_security TEXT NOT NULL,
    symbol TEXT NOT NULL,
    currency TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- the fold of its events
    phase TEXT NOT NULL CHECK (phase IN ('waiting','guarding','to-target','target','back-to-stop','to-market','firing','closing-for-sale','selling','closing','halted','ended')),
    updated_at TEXT NOT NULL
) STRICT;
INSERT INTO brackets_v25 (id, broker, broker_account, broker_security, symbol, currency, created_at, phase, updated_at)
    SELECT id, broker, broker_account, broker_security, symbol, currency, created_at, phase, updated_at FROM brackets;
DROP INDEX brackets_live;
DROP TABLE brackets;
ALTER TABLE brackets_v25 RENAME TO brackets;
CREATE INDEX brackets_live ON brackets(phase) WHERE phase <> 'ended';
