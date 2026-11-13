-- Whether an instrument is a fund (an ETF), as its source states it: 1 or 0 where a
-- record says (Wealthsimple's EXCHANGE_TRADED_FUND or EQUITY), null where none does.
-- The mapping's next version states it for every instrument a record names, so the
-- book's re-derivation fills it in; nothing infers it from a name.
ALTER TABLE instruments ADD COLUMN fund INTEGER CHECK (fund IN (0, 1));
