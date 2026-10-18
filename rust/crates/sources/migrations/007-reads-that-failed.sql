-- A read that failed, by what it was of (a listing's filer under a regulator, the
-- TSX's interlisted list) and its source: when it last failed, how many times in
-- a row, and why, so it is asked again on a growing rest (`market::grown_rest`),
-- never on every pass, and its failure is said until a read goes through (brief 42, 8).
CREATE TABLE read_failures (
    subject TEXT NOT NULL,
    source TEXT NOT NULL,
    failed_at TEXT NOT NULL,
    failures INTEGER NOT NULL,
    why TEXT NOT NULL,
    PRIMARY KEY (subject, source)
) STRICT;
