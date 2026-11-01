# Plan: every trade says whether it was long or short, and a written option whether it was covered

## For the owner to decide

Nothing open. The owner asked for long/short on the Trades page and for written options to say whether they were covered (2026-10-30), and left the design to the session: "there are many other trade journaling apps and other financial apps. there shouldn't be too much ambiguity when reviewing the top apps".

## Scope

The Trades table gains a **Side** column, and a trade's detail page states the same. Side reads `Long` or `Short` for every trade. For a short option it names what the writer held against it when it was written:
- a short call: `Covered call`, or `Uncovered call`;
- a short put: `Secured put`, or `Uncovered put`.

Touched:
- `SPEC.md` §2 Trade (a new definition, trade-69 onward) and §4 Trades (trades-1 gains the column; trades-10, the detail page);
- the engine (the coverage at the open);
- the wire (`Trade.coverage`);
- the page (`Trades.svelte`, `TradeDetail.svelte`).

Out of scope, on purpose:
- Spreads and other multi-leg strategies' names: Wealthsimple writes none.
- Live coverage on the Positions page: a holding's coverage today is a different question from what a trade was when it was written.
- Agents: they already get `direction`.

## The old app here

The Python app showed neither side nor coverage; nothing is carried over. The Rust engine already keeps each lot's direction (`engine::ledger::Direction`, `model::fifo` `open_direction`, sent on the wire as `Trade.openDirection`). The page doesn't show it.

## How the leading products do it

Read 2026-11-01:
- **Journals:**
  - Tradervue's and TraderSync's trade lists carry a Side column, Long or Short (tradervue.com/site/options-trading-tracker; luxalgo.com/blog/tradersync-trade-journal-review).
  - Neither documents an automatic covered or uncovered label; TradeZella leaves strategy names to the trader's own tags (help.tradezella.com/en/articles/5858193-applying-tags-to-trades).
- **Brokers:**
  - Brokers name what a written option is held against. Fidelity's option positions name a "Cash Covered Put": "a short put position in a cash account that is secured by setting aside cash equal to 100% of the exercisable value of the put contract(s)". Fidelity calls the rest uncovered (fidelity.com/webcontent/ap002390-mlo-content/20.04/help/learn_option_summary.shtml).
  - The industry's own terms: a call is covered when the writer holds the underlying, a put when cash is set aside; otherwise uncovered, also called naked (nasdaq.com/glossary/u/uncovered-put).
  - Wealthsimple writes only covered calls and secured puts. A covered call needs 100 shares of the underlying per contract (help.wealthsimple.com/hc/en-ca/articles/39258358639387). A secured put in a registered or cash account reserves "strike × 100 × contracts less the premium received" (help article 43198629134235, as brief 16 cites).

So the side follows the journals, as a column. The coverage follows the brokers' own terms, worked out by the journal because none of the journals does it. "Secured put" is Wealthsimple's name for a put backed by cash; "Uncovered" is Fidelity's, the clearer of the two words for the other case.

## The rule (SPEC.md §2 Trade, new)

- **Side:** the direction the trade opened in, `Long` or `Short`. It is already decided per lot (trade-2), and never changes within a round trip.
- **When coverage is decided:** a short option's coverage is decided once, at each contract's opening fill, from that account's book at that instant. A writer's later sale of the shares doesn't relabel a trade written covered, as a journal records what was done.
- **A short call** is covered by units of its underlying held long in the same account at that instant, `contract size × contracts` of them (trade-18: the size the terms state). Units already covering earlier calls still open in that account are left out, earlier opens first.
- **A short put** is secured by that account's cash in the option's currency at that instant: `strike × contract size × contracts − premium received`, net of what earlier secured puts still open there hold.
- **A trade of several contracts or several fills:**
  - it reads covered (or secured) only when every contract was;
  - otherwise it reads uncovered;
  - the detail page says how many were: `1 of 2 contracts covered`.
- **A roll:** each leg's coverage is decided at its own open, and the trade reads uncovered if any leg was.
- **Not known:** where the book can't say (a holding or cash figure waiting on a read, trade-68), the trade reads `Short` with no coverage word, and the detail page says why. Never a guess.
- **Brokers:** no broker is a case of its own. The rule reads only the book's own lots and cash, so a broker that allows uncovered writing gets `Uncovered`, and Wealthsimple's rules show up as covered and secured on their own.

## Approach

- **Engine:** the ledger already walks each account's lots and cash in time order (`engine::ledger`). At a short option lot's open it reads:
  - the underlying's long units in that account, less those taken by open covering calls;
  - the account's cash in the currency, less open secured puts' holds.

  It records each contract's coverage on the lot, as `Covered`, `Secured`, `Uncovered` or `Unknown`. A trade's coverage is the least of its lots'.
- **Wire:** `Trade.coverage: Option<Coverage>`, null for a long trade and for anything not an option. Generated `wire.ts` is regenerated.
- **Page:**
  - `Trades.svelte` gains a `Side` column after Symbol. It reads `Long`, `Short`, `Covered call`, `Secured put`, `Uncovered call` or `Uncovered put`, and sorts by that text.
  - The widths are rebalanced so that every width stays fixed (trades-5).
  - `TradeDetail.svelte` states the side, and for an option the count covered.
- **Unchanged:** no figure changes, no migration, no new read.

## Acceptance criteria

- [ ] Engine cases, their expected coverage written by an agent that had not read the engine:
  - a covered call (shares held);
  - a call of 2 contracts with 150 shares (uncovered, 1 of 2);
  - two calls, where the shares cover the first only (the second uncovered);
  - a secured put (cash ≥ strike × size × contracts − premium);
  - a put with too little cash (uncovered);
  - a put written while the cash read waits (unknown);
  - a roll whose far leg is uncovered;
  - a contract size other than 100.

  `cargo test -p bagholder-engine --test cases` green.
- [ ] `cargo test --workspace` green with no warnings, and clippy as CI runs it clean.
- [ ] `npm run check`, `npm test` and `npm run e2e` green. A browser test drives the Side column's text for each of the six values on the made-up book, and its sorting.
- [ ] The made-up book holds at least one covered call, one secured put and one uncovered option, so the screenshots show them.
- [ ] Rendered on the scratch server, on a copy of the book, at 1200 / 1340 / 1440 / 1680 px: no column overflowing at 1340 and above, and every Side value traced to the lot it came from.
- [ ] `SPEC.md` trades-1, trades-10 and the new §2 rule state the above, citing the sources here.

## Surfaces to check beyond the diff

- `web/src/lib/generated/wire.ts`;
- the wire fixtures' blessing;
- the routes golden if `Trade` is in it;
- the screenshot baselines of the Trades page;
- `model::fifo` (the second matcher, which must agree with the ledger on direction);
- the agents' trade tool, which keeps `direction` and gains `coverage` only if the wire type carries it there.

## Right to refuse

If the ledger can't see an account's cash at a lot's open without a second pass, stop and say so before building around it.

## Verification / Handoff

Filled in the PR that builds this.
