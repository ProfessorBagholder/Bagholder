-- Net worth by day, as the broker's net worth feature states it over its own
-- history (docs/plans/debts-held-outside-wealthsimple.md): every account of the
-- broker's and every outside account, one value a day, read whole and kept as
-- the newest read states it; `owed`, what the outside debts alone came to that
-- day (owed positive), where it was read.
CREATE TABLE net_worth_days (
    day TEXT PRIMARY KEY NOT NULL,
    amount TEXT NOT NULL,
    owed TEXT,
    currency TEXT NOT NULL,
    read_at TEXT NOT NULL
) STRICT;
