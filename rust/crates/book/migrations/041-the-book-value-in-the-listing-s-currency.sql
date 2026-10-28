-- Each position's book value in the listing's own currency, as Wealthsimple's
-- positions read states it asked with `currencyOverride: MARKET`
-- (`marketBookValue`), beside `book_value`, the same in CAD. None in a statement
-- read before this column: that read did not ask for it.
ALTER TABLE statement_units ADD COLUMN market_book_value TEXT;
ALTER TABLE statement_units ADD COLUMN market_book_value_currency TEXT;
