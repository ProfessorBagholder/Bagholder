-- The wrong codes typed on a sign-in's page, counted with the sign-in so a restart
-- never forgets them: the sign-in ends at the third (brief 81 §2). 0 for a sign-in
-- made before this column: no code was typed on its page.
ALTER TABLE agent_requests ADD COLUMN wrong_codes INTEGER NOT NULL DEFAULT 0;
