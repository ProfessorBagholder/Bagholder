-- What the broker states is kept when it changes and read by account
-- (docs/plans/stage-3-the-brokers-statements.md, brief 22). A statement of an
-- account's cash, buying power or value now becomes a period: one row per value,
-- with the first and last moment the broker stated it, the reads in between kept
-- in broker_reads as before. Runs of equal statements already kept are folded,
-- in the order of the reads that made them: a read's instant, then the order
-- the reads were written in, which tells two at one instant apart. Every instant the
-- broker check turns on is kept as an integer of nanoseconds, which sorts as
-- time (the text form's fraction varies in length and does not).
PRAGMA defer_foreign_keys = ON;

-- an instant's text (RFC 3339 in UTC, as the book writes it) as nanoseconds
CREATE TEMP TABLE instants (t TEXT PRIMARY KEY, ns INTEGER NOT NULL);
INSERT OR IGNORE INTO instants (t, ns)
    SELECT t, unixepoch(substr(t, 1, 19)) * 1000000000
        + CASE WHEN substr(t, 20, 1) = '.' THEN CAST(substr(substr(t, 21, length(t) - 21) || '000000000', 1, 9) AS INTEGER) ELSE 0 END
    FROM (SELECT at AS t FROM broker_reads
          UNION SELECT read_at FROM activity_reads
          UNION SELECT stated_at FROM statements WHERE stated_at IS NOT NULL
          UNION SELECT stated_at FROM buying_power
          UNION SELECT stated_at FROM account_values);

-- each read, and each read of an account's activity, at its instant
ALTER TABLE broker_reads ADD COLUMN at_ns INTEGER NOT NULL DEFAULT 0;
UPDATE broker_reads SET at_ns = (SELECT ns FROM instants WHERE t = broker_reads.at);
CREATE INDEX broker_reads_part ON broker_reads (connection_id, part, at_ns);
ALTER TABLE activity_reads ADD COLUMN at_ns INTEGER NOT NULL DEFAULT 0;
UPDATE activity_reads SET at_ns = (SELECT ns FROM instants WHERE t = activity_reads.read_at);
CREATE INDEX activity_reads_complete ON activity_reads (account_id, complete, at_ns);

-- cash statements, folded: a run is the statements of one account, in the order
-- of their reads, whose cash in every currency and holds
-- (a missing amount equal only to a missing one) are the same as the one before
CREATE TEMP TABLE cash_rows AS
    SELECT s.rowid AS r, s.id, s.account_id AS a, s.stated_at, s.read_id, i.ns AS t, br.rowid AS q,
        (SELECT group_concat(quote(currency) || '=' || quote(amount), ',' ORDER BY currency) FROM statement_cash WHERE statement_id = s.id) AS cash,
        (SELECT group_concat(quote(record_id) || quote(kind) || quote(currency) || quote(instrument_id) || quote(amount) || quote(quantity) || quote(premium), ',' ORDER BY record_id) FROM statement_holds WHERE statement_id = s.id) AS holds
    FROM statements s JOIN instants i ON i.t = s.stated_at JOIN broker_reads br ON br.id = s.read_id WHERE s.kind = 'cash';
CREATE TEMP TABLE cash_runs AS
    SELECT *, sum(starts) OVER (PARTITION BY a ORDER BY t, q, r ROWS UNBOUNDED PRECEDING) AS run FROM (
        SELECT *, CASE WHEN lag(r) OVER w IS NOT NULL AND lag(cash) OVER w IS cash AND lag(holds) OVER w IS holds THEN 0 ELSE 1 END AS starts
        FROM cash_rows WINDOW w AS (PARTITION BY a ORDER BY t, q, r));
CREATE INDEX cash_runs_run ON cash_runs (a, run, t, q, r);

CREATE TABLE statements_v26 (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id),
    kind TEXT NOT NULL CHECK (kind IN ('cash', 'units')),
    -- the first moment the cash was stated, for cash
    stated_at TEXT,
    -- the day the units are stated as of, for units
    as_of_day TEXT,
    -- the read that first stated it
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    -- for cash: the first and last moment it was stated (the read's instant in
    -- nanoseconds, then the read's place among the reads), and the read that last
    -- stated it
    first_ns INTEGER,
    first_seq INTEGER,
    last_ns INTEGER,
    last_seq INTEGER,
    last_read_id TEXT REFERENCES broker_reads(id),
    CHECK ((kind = 'cash') = (stated_at IS NOT NULL)),
    CHECK ((kind = 'units') = (as_of_day IS NOT NULL)),
    CHECK ((kind = 'cash') = (first_ns IS NOT NULL)),
    CHECK ((kind = 'cash') = (first_seq IS NOT NULL)),
    CHECK ((kind = 'cash') = (last_ns IS NOT NULL)),
    CHECK ((kind = 'cash') = (last_seq IS NOT NULL)),
    CHECK ((kind = 'cash') = (last_read_id IS NOT NULL)),
    CHECK (first_ns < last_ns OR (first_ns = last_ns AND first_seq <= last_seq))
) STRICT;
INSERT INTO statements_v26 (id, account_id, kind, stated_at, as_of_day, read_id, first_ns, first_seq, last_ns, last_seq, last_read_id)
    SELECT id, account_id, kind, stated_at, as_of_day, read_id, first_ns, first_seq, last_ns, last_seq, last_read_id FROM (
        SELECT s.rowid AS r, s.id, s.account_id, s.kind, NULL AS stated_at, s.as_of_day, s.read_id, NULL AS first_ns, NULL AS first_seq, NULL AS last_ns, NULL AS last_seq, NULL AS last_read_id
        FROM statements s WHERE s.kind = 'units'
        UNION ALL
        SELECT k.r, k.id, k.a, 'cash', k.stated_at, NULL, k.read_id, k.t, k.q, l.t, l.q, l.read_id
        FROM cash_runs k JOIN cash_runs l ON l.a = k.a AND l.run = k.run
            AND l.r = (SELECT g.r FROM cash_runs g WHERE g.a = k.a AND g.run = k.run ORDER BY g.t DESC, g.q DESC, g.r DESC LIMIT 1)
        WHERE k.starts = 1)
    ORDER BY r;
DELETE FROM statement_cash WHERE statement_id NOT IN (SELECT id FROM statements_v26);
DELETE FROM statement_holds WHERE statement_id NOT IN (SELECT id FROM statements_v26);
DROP TABLE statements;
ALTER TABLE statements_v26 RENAME TO statements;
CREATE INDEX statements_cash ON statements (account_id, kind, first_ns, first_seq);
CREATE INDEX statements_units ON statements (account_id, kind, as_of_day);

-- buying power, folded the same way: the same amount in the same currency, or
-- the same reason it is unavailable
CREATE TEMP TABLE power_runs AS
    SELECT *, sum(starts) OVER (PARTITION BY a ORDER BY t, q, r ROWS UNBOUNDED PRECEDING) AS run FROM (
        SELECT *, CASE WHEN lag(r) OVER w IS NOT NULL AND lag(v) OVER w IS v THEN 0 ELSE 1 END AS starts FROM (
            SELECT b.rowid AS r, b.account_id AS a, b.stated_at, b.amount, b.currency, b.unavailable, b.read_id, i.ns AS t, br.rowid AS q,
                quote(b.amount) || quote(b.currency) || quote(b.unavailable) AS v
            FROM buying_power b JOIN instants i ON i.t = b.stated_at JOIN broker_reads br ON br.id = b.read_id)
        WINDOW w AS (PARTITION BY a ORDER BY t, q, r));
CREATE INDEX power_runs_run ON power_runs (a, run, t, q, r);
CREATE TABLE buying_power_v26 (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    -- the first moment it was stated
    stated_at TEXT NOT NULL,
    amount TEXT,
    currency TEXT,
    unavailable TEXT,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    first_ns INTEGER NOT NULL,
    first_seq INTEGER NOT NULL,
    last_ns INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    last_read_id TEXT NOT NULL REFERENCES broker_reads(id),
    CHECK ((amount IS NULL) = (currency IS NULL)),
    CHECK ((amount IS NULL) <> (unavailable IS NULL)),
    CHECK (first_ns < last_ns OR (first_ns = last_ns AND first_seq <= last_seq))
) STRICT;
INSERT INTO buying_power_v26 (account_id, stated_at, amount, currency, unavailable, read_id, first_ns, first_seq, last_ns, last_seq, last_read_id)
    SELECT k.a, k.stated_at, k.amount, k.currency, k.unavailable, k.read_id, k.t, k.q, l.t, l.q, l.read_id
    FROM power_runs k JOIN power_runs l ON l.a = k.a AND l.run = k.run
        AND l.r = (SELECT g.r FROM power_runs g WHERE g.a = k.a AND g.run = k.run ORDER BY g.t DESC, g.q DESC, g.r DESC LIMIT 1)
    WHERE k.starts = 1 ORDER BY k.r;
DROP TABLE buying_power;
ALTER TABLE buying_power_v26 RENAME TO buying_power;
CREATE INDEX buying_power_account ON buying_power (account_id, first_ns, first_seq);

-- what each account is worth now, folded the same way
CREATE TEMP TABLE value_runs AS
    SELECT *, sum(starts) OVER (PARTITION BY a ORDER BY t, q, r ROWS UNBOUNDED PRECEDING) AS run FROM (
        SELECT *, CASE WHEN lag(r) OVER w IS NOT NULL AND lag(v) OVER w IS v THEN 0 ELSE 1 END AS starts FROM (
            SELECT x.rowid AS r, x.account_id AS a, x.stated_at, x.amount, x.currency, x.read_id, i.ns AS t, br.rowid AS q,
                quote(x.amount) || quote(x.currency) AS v
            FROM account_values x JOIN instants i ON i.t = x.stated_at JOIN broker_reads br ON br.id = x.read_id)
        WINDOW w AS (PARTITION BY a ORDER BY t, q, r));
CREATE INDEX value_runs_run ON value_runs (a, run, t, q, r);
CREATE TABLE account_values_v26 (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    -- the first moment it was stated
    stated_at TEXT NOT NULL,
    amount TEXT NOT NULL,
    currency TEXT NOT NULL,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    first_ns INTEGER NOT NULL,
    first_seq INTEGER NOT NULL,
    last_ns INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    last_read_id TEXT NOT NULL REFERENCES broker_reads(id),
    CHECK (first_ns < last_ns OR (first_ns = last_ns AND first_seq <= last_seq))
) STRICT;
INSERT INTO account_values_v26 (account_id, stated_at, amount, currency, read_id, first_ns, first_seq, last_ns, last_seq, last_read_id)
    SELECT k.a, k.stated_at, k.amount, k.currency, k.read_id, k.t, k.q, l.t, l.q, l.read_id
    FROM value_runs k JOIN value_runs l ON l.a = k.a AND l.run = k.run
        AND l.r = (SELECT g.r FROM value_runs g WHERE g.a = k.a AND g.run = k.run ORDER BY g.t DESC, g.q DESC, g.r DESC LIMIT 1)
    WHERE k.starts = 1 ORDER BY k.r;
DROP TABLE account_values;
ALTER TABLE account_values_v26 RENAME TO account_values;
CREATE INDEX account_values_account ON account_values (account_id, first_ns, first_seq);

DROP TABLE instants;
DROP TABLE cash_rows;
DROP TABLE cash_runs;
DROP TABLE power_runs;
DROP TABLE value_runs;
