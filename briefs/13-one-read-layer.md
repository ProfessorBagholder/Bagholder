# Brief 13: why screens still open empty, and the one fix

**Reviewed:** PR #323 (`docs/plans/detail-pages-never-empty.md` at `9e8ccf21`), and the page's reads on `master` at `6ce3d5ff`.

**Verdict on #323: Stop.**
- It fixes the next screen on the list the same way #321 and #322 fixed theirs, and the screens after it would each need another PR.
- Replace it with the plan below, built as one PR.
- Nothing in it is the owner's to decide.

## Why it keeps happening

**There is no one place the page reads through.**
- The tabs read through the subscriptions and the browser store (`subs.svelte.ts`, `live.svelte.ts`, `kept.ts`).
- Nine other modules make their own reads, and `api.ts` keeps a second, memory-only cache (`lookup`). The nine:
  - `listing.svelte.ts`, `ui.svelte.ts`, `state.svelte.ts`;
  - `markets/News.svelte`, `markets/Shorts.svelte`, `markets/quotes.svelte.ts`;
  - `trade/chart.ts`, `trade/shorts.svelte.ts`, `trade/discStore.svelte.ts`.
- Seven components draw their own loading state whenever their own memory is empty.

So "keep the last value" has been built screen by screen:
- #321: the tabs on reload;
- #322: the tabs on first open;
- #323: the trade and holding pages.

Still left after #323:
- the Markets cards: Fear & Greed reads "Reading…" whenever its memory is empty (`markets/FearGreed.svelte:46`, `:119`); News for a listing; short interest;
- the Orders panel (`orders/OrdersPanel.svelte:36`);
- the filter's lists.

**Some screens throw away what they have.**
- The Disclosures card shows its loading state while it re-reads, even with a list in hand (`trade/Disclosures.svelte:52`).
- Every open of a trade or holding page makes the server fetch that listing's filings from SEDAR+ and EDGAR again, changed or not. `trade/discStore.svelte.ts:81` sends `refresh: true`, and `server/src/feeds.rs:1875` obeys it.

**My part.**
- Brief 10 kept the plan's "a tab never visited loads nothing" and told CTO to drop the browser store. Together they produced the empty first opens.
- The owner reversed both, on 2026-09-27 and 2026-09-28.

## The plan: one read layer

The pattern is stale-while-revalidate (RFC 5861):
- every read goes through one cache, keyed by what is read;
- a component asks the cache, never the server;
- the cache answers at once with the last value it holds, asks the server in the background, and replaces only what changed;
- a loading state exists only for a key the cache has never held.

The owner's rule then holds for every screen by construction, not one screen at a time.

**No new library.** It is a pattern, not a framework, and the page already has most of it. Build it on `subs.svelte.ts`, `live.svelte.ts` and `kept.ts`.

**Required:**

1. **One read function for the whole page.** It is the subscriptions' `use`, or one beside it on the same store.
   - Every GET goes through it:
     - the tab documents, and the trade and holding documents;
     - executions, chart bars, short interest, disclosures, news and Fear & Greed;
     - the quotes shown;
     - orders, notes and the filter's lists.
   - Each key is kept in the browser store (`kept.ts`) with its version. On the next open and the next page load it is drawn at once, then checked against the server's ETag or version, so an unchanged answer moves nothing.
   - Writes stay as calls.
   - The keys whose answer must not outlive the moment are marked at the key, in one list: a search the person types, and the ticket's live quote.
   - `lookup`'s memory-only cache goes.

2. **No placeholder where a value exists.**
   - A component never draws a placeholder while its key holds a value.
   - A placeholder is drawn only for a key never held.
   - While a newer answer is read, the old one stays on screen: no "Reading…" and no shimmer over existing data.

3. **The page never forces an outside read.**
   - `refresh: true` goes.
   - The server refreshes a source by its own staleness rule (`filings_stale` for filings) and sends what changed.

4. **What exists is drawn at once; what doesn't is read when needed, then kept.**
   - A trade or holding page draws from its row (#323's step 1, kept) and its executions from the book.
   - A price history the app has never read, for a trade never opened, is read the first time its page opens and kept from then on. A closed session's bars are never read again.
   - Nothing is read ahead for every trade. The owner's rule is to load data when it is needed (2026-09-28), and only the past closes a chart shows (2026-09-24).

5. **Held by two tests over every screen, not one test per screen** (`CLAUDE.md`, a rule for any input):
   - **A scan:** it fails on any GET made outside the read function.
   - **A browser walk:**
     - It opens the app once, then reloads with the server's answers held back.
     - It visits every screen the router and the panels can show: every tab, card, detail page, panel and overlay.
     - It fails if a placeholder is ever inserted where the first open had a value.
     - When the server's answers are released unchanged, it fails if any element changes (the MutationObserver check in `web/e2e/dataflow.spec.ts`).
     - Its list of screens is generated from the router's and panels' own lists, so a screen added later is walked without anyone writing a test for it.

## How the owner hears from CTO

**Add this to the top of `CLAUDE.md`, as the owner's rule.** No file has it today, and step 4 of "How changes land" invites the opposite:

> **Work until it is done, then report once.** Build, verify (tests, the scratch server, the browser) and fix until every acceptance criterion is met before telling the owner anything. The owner hears from you once, when the work is complete: what changed, in a few plain lines. No progress reports, no play-by-play, no steps for the owner to test: testing is your job. The only exceptions are a decision only the owner can make (one question, nothing else) and an action only the owner can do (a sign-in), each named alone.

**Fix `CLAUDE.md` where it still points at `svelte-migration`, a branch that no longer exists:**
- the gate;
- "How changes land", steps 1 and 5.

Say where plans and PRs go now.

## For the owner

Nothing.
