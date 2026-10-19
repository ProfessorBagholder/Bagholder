-- The browsers that asked to be told by Web Push while no Bagholder tab is open
-- (docs/plans/notifications-through-the-browser.md): each subscription's endpoint,
-- the push service's capability URL, with the keys its messages are encrypted to
-- (RFC 8291). A browser holds a subscription to this copy alone, on this data
-- folder, so no other copy can notify the person.
CREATE TABLE push_subscriptions (
    endpoint TEXT PRIMARY KEY NOT NULL,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    made_at TEXT NOT NULL
) STRICT;
