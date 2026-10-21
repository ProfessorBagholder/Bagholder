-- Each outside account's balance by day, as the broker's net worth feature states
-- it of that account alone (docs/plans/debts-held-outside-wealthsimple.md): a
-- debt's owed positive, an asset's value positive, in the currency stated. Net
-- worth by day is worked out from these and the broker's own accounts' values, so
-- the broker's own aggregate by day (migrations 34 and 35) is no longer kept.
DROP TABLE net_worth_days;
CREATE TABLE outside_days (
    entity_id TEXT NOT NULL,
    day TEXT NOT NULL,
    amount TEXT NOT NULL,
    currency TEXT NOT NULL,
    read_at TEXT NOT NULL,
    PRIMARY KEY (entity_id, day)
) STRICT;
