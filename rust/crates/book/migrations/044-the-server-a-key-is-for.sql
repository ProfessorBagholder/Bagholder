-- The server a key was made for (`https://{host}`): it is taken only there, as a
-- sign-in's token is only at its resource. A key made before this column was made
-- when Bagholder had one address, kept in `agents.host`: that one is its server.
-- A key made while no address was set up has none until it is first used: the
-- server it is used through is its own.
ALTER TABLE agent_keys ADD COLUMN at TEXT;
UPDATE agent_keys SET at = (SELECT 'https://' || value FROM settings WHERE key = 'agents.host');
