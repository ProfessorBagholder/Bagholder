-- What a statement of cash keeps is read by key (docs/plans/stage-3-the-brokers-statements.md):
-- the holds against one account, and an order by the broker's id for it, each through
-- an index, so storing one account's statement does not read every account's holds or
-- every order the app has kept.
CREATE INDEX record_holds_account ON record_holds (account_id);
CREATE INDEX orders_by_broker_id ON orders (broker_id) WHERE broker_id IS NOT NULL;
