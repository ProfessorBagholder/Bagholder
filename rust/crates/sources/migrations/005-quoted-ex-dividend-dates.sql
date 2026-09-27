-- The ex-dividend date a listing's quote states (TMX's `exDividendDate`), kept
-- with the quote it came with: the Ex-Div of a payer no declared record serves
-- (SPEC.md §5, Cashflow Positions). One per instrument, replaced by each quote
-- read that states one and removed by one that states none.
CREATE TABLE quoted_ex_dividends (
    instrument_id TEXT NOT NULL PRIMARY KEY,
    source TEXT NOT NULL,
    ex_date TEXT NOT NULL,
    received_at TEXT NOT NULL
) STRICT;
