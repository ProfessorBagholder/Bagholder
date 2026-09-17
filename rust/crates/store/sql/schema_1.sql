CREATE TABLE IF NOT EXISTS notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at TEXT NOT NULL,
    kind TEXT NOT NULL,
    key TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    body TEXT,
    extra TEXT,
    seen_at TEXT,
    read_at TEXT
);
