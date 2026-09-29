# Brief 14: brief 13 as built

**Reviewed:** `master` at `734221d7`, #324 to #326.

**Verdict: accepted, with one fix.**

## What meets the owner's requirement

- **One read path.** Every read goes through `read` (`web/src/lib/reads.svelte.ts`). No other module makes a read of its own, and `lookup`'s separate cache is gone.
- **Every screen is walked.**
  - `web/e2e/everyscreen.spec.ts` takes the screens from the router's tabs and the page's own panels and dialogs, plus a trade page, a holding page and a listing page.
  - It fails if a value on screen is replaced by a loading state, or if an unchanged answer moves anything.
- **No forced outside read.** Filings are fetched again from their sources only when the person presses "Re-read disclosures". A placeholder stands only when nothing is held.
- **Clear all.** The served page carries the current book's id (`server/src/http/assets.rs:66`), so a browser that was closed during Clear all draws nothing of the old book (`web/src/lib/live.svelte.ts:176-178`).

## The fix

**`call` still accepts reads** (`web/src/lib/api.ts:104`).
- **The risk:** a future screen can read the server around `read`, and the loading states come back where no walk step reaches them.
- **Fix:** type `call` to write routes only. The type checker then refuses any read through it.

## Recorded

- **Charts are read ahead.** Every trade's and holding's chart is read ahead once and kept (#326), not on the first open of its page as brief 13 said.
  - It follows the owner's own words: never "nothing saved to show yet".
  - It costs one paced read per trade, ever.
  - Accepted. Record it in `docs/decisions.md`.
- **`CLAUDE.md`:** brief 13's changes wait on the owner. CTO's session may not edit that file, and nobody else edits it on CTO's behalf.
