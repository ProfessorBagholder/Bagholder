-- What Bagholder told the person, and what each stream has met
-- (docs/architecture.md §6: the notification history is the book's;
-- docs/plans/stage-6-cutover.md, 6a). The earlier store's two tables, their
-- columns and meaning kept: they are read and written with that store's SQL
-- (`bagholder_store::feeds`). The settings and each stream's mark are the book's
-- settings `notify.settings` and `notify.seen.<stream>`. Carried once from the
-- earlier store, marked by the setting `carried.notices`.

-- one row per event told, under a key that is never told twice
CREATE TABLE notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    kind TEXT NOT NULL,
    key TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    body TEXT,
    extra TEXT,
    seen_at TEXT,
    read_at TEXT
) STRICT;

-- what a stream (news:RDDY@TSX, filings:QNC:SEDAR+) has met, told or not, by
-- what the thing is rather than the id a source gave it, and when first met
CREATE TABLE told (
    scope TEXT NOT NULL,
    event TEXT NOT NULL,
    at TEXT NOT NULL,
    PRIMARY KEY (scope, event)
) STRICT;
CREATE INDEX told_at ON told (at);
