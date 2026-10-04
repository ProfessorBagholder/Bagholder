# Brief 18: the reserved-cash plan at `eb7a0f05`: no objection to the source, one addition

**Read:** CTO's correction of 2026-10-04 (the 2026-10-03 finding was a query that filtered `PENDING` before taking a record's newest revision; both option rows moved to `EXPIRED` at the first full read after each lapsed; the 2.0.14 read-back agrees, the 09-10 stock sell is `UNTIL_CANCEL` to 2026-12-08 and holds no units).

## Verdict

No objection. With the stale-row finding withdrawn, the activity feed's pending rows are the better source, not merely an acceptable one: they are records the book already keeps with revisions; the broker states the held amount on the row, so nothing is multiplied out; they are read in the same full pull as the cash the check pairs them with; and they do not put the broker pull on the legacy order client the cutover stage deletes. Brief 17's other changes stand as CTO lists them: the documented rule's branch (a working buy holds what the broker states, a market buy or IPO bid an unstated amount), unverified kinds unstated and the check pending while one is open, the secured-put arm from the article marked unverified, holds keyed by record. Issue #346 withdrawn is right.

## One addition before the build

**A partly filled buy.** The fill is booked from its own completed row; the pending row stays `PENDING` for the rest. Whether that row's `amount` is then the remainder or still the whole order is not known from any recorded reply. If it is the whole, the book's cash less the hold is too low by the filled part and the check says a difference that is not there. Settle it from a recorded or live partial fill if one exists; where it cannot be settled, a pending buy row with a completed fill against the same order is a hold of unknown size and the currency's check is pending. One engine case for it, expected figures written without reading the engine.

## For the owner

Nothing to decide. Nothing on screen changes.
