-- A closed account is not stated as holding nothing (2.10.3, as 2.9.0): the
-- statements 2.10.0 to 2.10.2 wrote for every closed account at each sync and
-- each balances pass, a cash or units statement with no row under it, go, so
-- the broker check covers closed accounts as 2.9.0's did. A statement from
-- when the account was open keeps its rows, and stays.
DELETE FROM statements
 WHERE kind IN ('cash', 'units')
   AND account_id IN (SELECT id FROM accounts WHERE status = 'closed')
   AND id NOT IN (SELECT statement_id FROM statement_cash)
   AND id NOT IN (SELECT statement_id FROM statement_units)
   AND id NOT IN (SELECT statement_id FROM statement_holds);
