-- The orders Bagholder sends and the brackets that guard their fills, each with
-- its log (docs/architecture.md §11, docs/plans/stage-4-execution.md). An order's
-- or a bracket's state is the fold of its events; the state columns here are that
-- fold, kept for finding live ones by state, and rebuilt from the log.

-- A bracket: the stop and the target held for an entry.
CREATE TABLE brackets (
    id TEXT PRIMARY KEY,
    -- where its exits trade: the broker's own ids, as they are sent
    broker TEXT NOT NULL,
    broker_account TEXT NOT NULL,
    broker_security TEXT NOT NULL,
    symbol TEXT NOT NULL,
    currency TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- the fold of its events
    phase TEXT NOT NULL CHECK (phase IN ('waiting','guarding','to-target','target','back-to-stop','to-market','firing','closing-for-sale','closing','halted','ended')),
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX brackets_live ON brackets(phase) WHERE phase <> 'ended';

-- An order Bagholder wrote, before anything was sent: its id is the external id
-- the broker is sent and read back by.
CREATE TABLE orders (
    id TEXT PRIMARY KEY,
    broker TEXT NOT NULL,
    broker_account TEXT NOT NULL,
    broker_security TEXT NOT NULL,
    symbol TEXT NOT NULL,
    currency TEXT NOT NULL,
    side TEXT NOT NULL CHECK (side IN ('buy','sell')),
    order_type TEXT NOT NULL CHECK (order_type IN ('market','limit','stop','stop-limit')),
    quantity TEXT NOT NULL,
    limit_price TEXT,
    stop_price TEXT,
    time_in_force TEXT NOT NULL CHECK (time_in_force IN ('day','until-cancel')),
    -- the bracket it is the entry or an exit of, and which
    bracket_id TEXT REFERENCES brackets(id),
    role TEXT CHECK (role IN ('entry','stop','target','market')),
    -- the request as it was sent
    request TEXT NOT NULL,
    created_at TEXT NOT NULL,
    -- the fold of its events
    state TEXT NOT NULL CHECK (state IN ('dry','sending','unconfirmed','pending','partly-filled','filled','cancelling','cancelled','expired','rejected','failed')),
    broker_id TEXT,
    filled TEXT NOT NULL,
    average TEXT,
    why TEXT,
    code TEXT,
    -- what the broker last stated of it: its price, quantity and when it lapses
    stated_price TEXT,
    stated_quantity TEXT,
    expires_at TEXT,
    updated_at TEXT NOT NULL,
    CHECK ((bracket_id IS NULL) = (role IS NULL))
) STRICT;

CREATE INDEX orders_in_flight ON orders(state) WHERE state IN ('sending','unconfirmed','pending','partly-filled','cancelling');
CREATE INDEX orders_by_bracket ON orders(bracket_id, role) WHERE bracket_id IS NOT NULL;
CREATE INDEX orders_by_created ON orders(created_at);

-- What happened to an order, in order: who asked, what it was, and why an event
-- that was not a move from where it fell was refused.
CREATE TABLE order_events (
    order_id TEXT NOT NULL REFERENCES orders(id),
    seq INTEGER NOT NULL,
    at TEXT NOT NULL,
    asker TEXT NOT NULL,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    refused TEXT,
    PRIMARY KEY (order_id, seq)
) STRICT;

CREATE TABLE bracket_events (
    bracket_id TEXT NOT NULL REFERENCES brackets(id),
    seq INTEGER NOT NULL,
    at TEXT NOT NULL,
    asker TEXT NOT NULL,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    refused TEXT,
    PRIMARY KEY (bracket_id, seq)
) STRICT;
