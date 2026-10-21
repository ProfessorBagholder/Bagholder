-- What the debts held outside came to each day (owed positive), beside the day's
-- net worth, where it was read (docs/plans/debts-held-outside-wealthsimple.md).
-- The days held are cleared, so the next read states both.
ALTER TABLE net_worth_days ADD COLUMN owed TEXT;
DELETE FROM net_worth_days;
