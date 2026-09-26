-- The units a payment was paid on, where the source states them (a Wealthsimple
-- dividend states the units held): what the Distribution history's Qty and
-- per-unit amount read. Never a change to the position, which is `quantity`.
ALTER TABLE transactions ADD COLUMN paid_on TEXT;
