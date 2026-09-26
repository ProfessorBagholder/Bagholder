-- What the source states units were worth as they moved in (Wealthsimple's amount
-- on a coin moved in from outside): the arrival's cost where the person states
-- none. Never cash that moved, which is `cash`.
ALTER TABLE transactions ADD COLUMN value TEXT;
ALTER TABLE transactions ADD COLUMN value_currency TEXT;
