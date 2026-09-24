-- Whether a series of the Bank's rates has ended, as its source states it (the
-- noon archives stopped on 2017-04-28; the Bank marks a daily series it no longer
-- publishes as a "historical series"). A day after an ended series' last day, and
-- a day between two series, is one no source holds a published rate for; a day
-- after a series still published is one not read yet.
ALTER TABLE fx_series ADD COLUMN ended INTEGER NOT NULL DEFAULT 0 CHECK (ended IN (0, 1));
