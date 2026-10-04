-- The broker's order ids a record names (docs/plans/broker-check-reserved-cash.md,
-- brief 18): derived by the mapping like the record's transactions, replaced with
-- each derivation, so a hold on a working order is known to have a fill against
-- the same order, which leaves the hold's size unstated.
CREATE TABLE record_orders (
    record_id TEXT NOT NULL REFERENCES source_records(id),
    order_id TEXT NOT NULL,
    PRIMARY KEY (record_id, order_id)
) STRICT;
CREATE INDEX record_orders_order ON record_orders (order_id);
