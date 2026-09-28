-- What the broker states each account is worth now (Wealthsimple's
-- `financials.currentCombined.netLiquidationValue`, read with the accounts on
-- every sync): the account's net asset value, as the original app showed it.
-- Its daily history (`account_days`) is a day behind and is the equity curve's.
CREATE TABLE account_values (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    stated_at TEXT NOT NULL,
    amount TEXT NOT NULL,
    currency TEXT NOT NULL,
    read_id TEXT NOT NULL REFERENCES broker_reads(id),
    PRIMARY KEY (account_id, stated_at)
) STRICT;
