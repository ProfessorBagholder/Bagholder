-- Net worth by day, as the broker's net worth feature states it over its own
-- history (docs/plans/debts-held-outside-wealthsimple.md): every account of the
-- broker's and every outside account, one value a day, read whole and kept as
-- the newest read states it.
CREATE TABLE net_worth_days (
    day TEXT PRIMARY KEY NOT NULL,
    amount TEXT NOT NULL,
    currency TEXT NOT NULL,
    read_at TEXT NOT NULL
) STRICT;
