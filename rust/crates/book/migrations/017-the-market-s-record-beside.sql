-- The market's record of a payer read beside its company's (SPEC.md §5,
-- Cashflow Positions, Ex-Div and Pay Day): a fund company's publication lists a
-- distribution days after the fund declares it, while the exchange's record
-- (TMX) lists it the day it is declared. The rate and schedule stay the
-- company's; the market's record is read only for the next distribution still
-- to be paid while the company's has not listed it. Every read stored before is
-- the payer's record the figures come from (`payer`).
ALTER TABLE declared_reads ADD COLUMN role TEXT NOT NULL DEFAULT 'payer' CHECK (role IN ('payer', 'market'));
