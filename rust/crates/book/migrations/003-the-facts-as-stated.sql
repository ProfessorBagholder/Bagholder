-- The facts as their sources state them (docs/plans/stage-3a-sources.md).

-- A distribution is what its payer stated: its cash per unit, and where the
-- payer states one, the part reinvested per unit (a year-end capital gain paid
-- in units, a reinvested distribution). No kind is invented for it. A row
-- stage 2 stored as non-cash was wholly reinvested: its amount is the
-- reinvested part and it paid no cash.
CREATE TABLE declared_distributions_v3 (
    read_id INTEGER NOT NULL REFERENCES declared_reads(id),
    ex_date TEXT NOT NULL,
    record_date TEXT,
    pay_date TEXT,
    amount TEXT NOT NULL,
    reinvested TEXT,
    currency TEXT NOT NULL CHECK (length(currency) = 3)
) STRICT;
INSERT INTO declared_distributions_v3(read_id, ex_date, record_date, pay_date, amount, reinvested, currency)
    SELECT read_id, ex_date, record_date, pay_date,
           CASE kind WHEN 'non-cash' THEN '0' ELSE amount END,
           CASE kind WHEN 'non-cash' THEN amount END,
           currency
    FROM declared_distributions;
DROP INDEX declared_distributions_read;
DROP TABLE declared_distributions;
ALTER TABLE declared_distributions_v3 RENAME TO declared_distributions;
CREATE INDEX declared_distributions_read ON declared_distributions (read_id);

-- Each series of the Bank's rates for a currency, from each source that holds
-- one (the Bank's daily average, its noon rate until 2017, Statistics Canada's
-- archive of the noon rate before that), with its first and last observation
-- day: a day before a currency's oldest series is one no source holds a
-- published rate for. The rows before this version named only a currency; they
-- are read again on the next read of the series list, which is where they came
-- from.
DROP TABLE fx_series;
CREATE TABLE fx_series (
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    source TEXT NOT NULL,
    first_day TEXT NOT NULL,
    last_day TEXT NOT NULL,
    received_at TEXT NOT NULL,
    PRIMARY KEY (currency, source),
    CHECK (first_day <= last_day)
) STRICT;
