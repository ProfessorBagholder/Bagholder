# Plan: a trade or holding page opens on its last state, never on nothing

A done-contract for a heavy lift (the gate in `CLAUDE.md`): it changes the page's data layer for every on-demand read.

## For the owner to decide

Nothing open. The owner decided on 2026-09-28 that no screen ever opens with nothing except the very first open before Wealthsimple or a CSV (`docs/decisions.md`); this plan applies that decision to the one kind of screen #322 did not reach.

## Scope

The trade page (Trades → a trade) and the holding page (Portfolio → a holding): their figures, their executions, their price chart, their short interest and their disclosures. After #322 every tab and Markets card opens on its last state; these pages still open on a placeholder the first time each is opened in a page load, and their chart, executions, short interest and disclosures are held only in memory, so every restart reads them again with nothing on screen meanwhile. Out of it: the Markets listing page for a listing the book never held (it has no row to draw from, and is reached by a search the person types), and the order ticket's live quote (a quote is only ever the live one).

## The old app here

`ledger.html` held every trade in the page at once, so a trade page drew its figures at once; its chart bars were read on opening and not kept. Nothing is carried over from it: the figures come from the row the new page already holds, and the reads are kept by the rule `docs/architecture.md` §13 already states ("everything it loaded on demand … each with its version") and that the page never built.

## How the leading products do it

- Stale-while-revalidate: a stored answer is served at once while a fresh one is fetched in the background, without blocking (IETF RFC 5861, https://www.rfc-editor.org/rfc/rfc5861, read 2026-09-28).
- Data for a screen not yet opened is fetched ahead of navigation, declared up front rather than when the screen mounts, since fetching in components leads to request waterfalls (TanStack Query, Prefetching guide, https://tanstack.com/query/latest/docs/framework/react/guides/prefetching, read 2026-09-28).
- Trading journals (TradeZella, Tradervue, TraderSync, Edgewonk) document no caching behaviour; nothing to check against there.

## Open questions

None. The trade count the first-open reads scale with is known: the owner's book holds 322 trades (the trades document's total, 2026-09-28), and the server paces every outside source itself (`SPEC.md` §2, Market data), so the reads below cannot outrun a source.

## Approach

1. **The figures, from the row.** A trade or holding page is drawn from its row as soon as it opens: the trade from the trades document, the holding from the positions document (`holdingAsTrade`), in memory or kept (`web/src/lib/kept.ts`), until `trade:<id>` answers and replaces it in place. The rows are the same `Trade` and `Position` types the detail document carries (`web/src/lib/generated/figures.ts`), so nothing is converted. To have every trade's row, the trades document is kept once whole (`limit` = its `total`) as the book becomes known, as `everyScreen` does for the tabs, so a trade far down the list has its row too.
2. **Every on-demand read kept between opens.** `lookup()` (`web/src/lib/api.ts`) and the executions read (`loadDetail`, `web/src/lib/state.svelte.ts`) write each answer into the browser's store (`kept.ts`, the same store and book id, so Clear data clears it) and give the kept answer at once on the next ask, then ask the server and replace it only where it differs. The chart (`web/src/lib/trade/chart.ts`), short interest (`web/src/lib/trade/shorts.svelte.ts`) and disclosures (`web/src/lib/trade/discStore.svelte.ts`) read through these, so each draws its last state first. An answer the server marks pending is still not kept as final, as today.
3. **Every trade and holding read once ahead.** As the book becomes known, each trade and holding the browser has kept nothing of has its executions and its chart (at the timeframe its page opens on, `chartTfFor`) read once in the background and kept, one at a time, so the first opening of any of them draws at once. Short interest and disclosures are read ahead only for holdings (a few dozen symbols); for a closed trade's symbol they are read when its page opens, and shown from the kept answer on every opening after.
4. **Subscriptions before drawing**, as #322 did for the tabs: the detail page's reads start in `$effect.pre`.

`SPEC.md` does not change: it says what the pages show, not when they are read. `docs/architecture.md` §13 already requires on-demand reads to be kept between opens; the plan builds that.

## Acceptance criteria

- [ ] `npm run check`, `npm test` and `npm run e2e` green in `web/`.
- [ ] A browser test: after a first open, with the server's answers held back, a trade page never opened is drawn with its figures, executions and chart, and no placeholder or arrival animation is drawn at any point (the MutationObserver check `web/e2e/dataflow.spec.ts` uses).
- [ ] A browser test: after a restart of the page, with the server held back, a trade page opened before draws its chart, executions, short interest and disclosures as they were.
- [ ] A browser test: a kept answer that differs from the server's is replaced in place once the server answers; one that is the same leaves every element untouched (a DOM-mutation check).
- [ ] A unit test on `lookup()`: a kept answer is given at once and the server's replaces it; a pending answer is never kept as final.
- [ ] On a copy of the owner's book on the scratch server: every trade in the list and every holding opened with the server held back draws whole; the first-open reads finish, and no outside source reports a refusal for them.

## Surfaces to check beyond the diff

`web/src/lib/api.ts` (`lookup` is used by symbol search and listings too: search answers must not be kept between opens), `web/src/lib/kept.ts` (store size: 322 trades' executions and bars), `docs/architecture.md` §13, `docs/decisions.md` 2026-09-28.

## Right to refuse

If a detail document's fields differ from the list row's in a way the page shows (a figure only the detail carries), the page draws that part from the kept detail only, and this is reported rather than a figure drawn from a different source.

## Anti-stub self-check

To be initialled at build time.

## Verification

To be filled at build time.

## Handoff

Not started; the plan waits on the gate.
