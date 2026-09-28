-- What the broker states each position of a statement of units is worth
-- (Wealthsimple's `totalValue`): a coin's amount the broker values at less
-- than its smallest sale is dust by the broker's own figure, with no price
-- read for it.
ALTER TABLE statement_units ADD COLUMN value TEXT;
ALTER TABLE statement_units ADD COLUMN value_currency TEXT;
