# Plan: book the coin units the broker's own positions show and no row states, so a sale never takes more than the book holds

A done-contract for a heavy lift (the gate in `CLAUDE.md`): it changes how a figure is defined (the cost of units no row states) and adds records of a new kind.

## For the owner to decide

1. **What units that no Wealthsimple row states cost, once the broker's positions show them.** On your crypto account, two sales took more than the book held: Bitcoin on 2024-11-06 by 0.000023 (about $2.46 at the sale's price) and Polkadot on 2024-12-18 by 0.136308 (about $1.59). Wealthsimple's own positions held those units; neither its activity feed nor its statements say where they came from (the statements round transfers to four places and list fewer staking rewards than the feed). Today the header says each sale "took more than the book held", and the units beyond count nothing to P&L, as `SPEC.md` has it. The choice is what they cost once booked:
   - **Their market value the day the positions first show them** (recommended). This is how a coin that arrives with no purchase is valued everywhere else: a staking reward is income at its market value when received, and that value is its cost (CRA practice as tax guides state it). The sale's P&L then counts only what the units gained since they arrived. It needs the coin's close that day, which the app already reads for charts.
   - **Nothing.** What Koinly assumes for any sale beyond what it can see ("a cost basis of $0 … acquired for free"); the whole sale value of those units becomes gain. Simpler, but it makes up a cost of nothing.
   - **Leave it as it is.** The header keeps both lines.

   Without a decision nothing is built.

## Scope

A coin sale or transfer out that takes more units than the book holds, in an account whose broker states positions by day. The pull reads the broker's positions to find the first day the book's units fall short of them before that sale, and books the shortfall there as units arriving, at the cost decided above. Touches `SPEC.md` §2 (a sale of more units than the book holds; dust) and §4 (the header's "took more than the book held" sentence, which then only remains where the broker's positions do not cover the day). Out of scope: shares and contracts (a share's missing units are a corporate event's, read from its entitlements already); cash.

## The old app here

The old app had no broker check, so it said nothing about these sales, and counted their proceeds against whatever lots it held. Nothing is carried over.

## How the leading products do it

- Koinly: a sale of more than the imported history holds is given a cost of zero and a "missing purchase history" warning; the fix it recommends is a deposit of exactly the missing amount just before the sale ([Koinly Help Center, "We have assumed a cost of zero for some assets"](https://support.koinly.io/en/articles/9490035-we-have-assumed-a-cost-of-zero-for-some-assets) and ["Missing purchase history for XYZ"](https://support.koinly.io/en/articles/9490037-missing-purchase-history-for-xyz), read through search 2026-09-27; the pages refuse direct reads).
- Coins received with no purchase (staking rewards) are income at their market value in CAD when received, which becomes their cost ([Questrade, "Tax on crypto in Canada"](https://www.questrade.com/learning/accounts-taxes/tax-on-crypto-canada), [TokenTax, "Guide to crypto taxes in Canada"](https://tokentax.co/blog/guide-to-crypto-taxes-in-canada), read 2026-09-27; both restate CRA practice).
- Journals (TradeZella, Tradervue, TraderSync) import fills only and have no positions reconciliation for coins to compare; nothing to follow there.

The recommendation follows the tax practice for coins that arrive with no purchase, over Koinly's zero, because it measures the gain the units actually made.

## Open questions

- **Which day the units arrived.** Objective: date the booking so the cost is the market's that day. Known: the Wealthsimple adapter reads positions for any past day (`BrokerAdapter::units`, used by the mapping's `changed_between`); the shortfall exists on the sale's day. Best course: halve the span between the last day the book and the positions agreed and the sale, reading positions per halving (about eleven reads for three years), since each read is one request and the positions are the only record that states it.

## Approach

`bagholder_broker::pull` after the units read: for each "beyond" the engine reports (`figures.matched.beyond`) on a coin, find the arrival day as above and book a record of the statement mapping's source with a new kind (`units-arrived`: account, instrument, day, units, the positions' two days), kind `TransferIn` with no cash and its value per the decision. `SPEC.md` changes in the same commit.

## Acceptance criteria

- [ ] `cargo test --workspace` green with tests: a shortfall found and booked on its first day; none where the positions never show it; the same pull again books nothing.
- [ ] An engine case whose expected figures were written by an agent that had not read the engine: a sale after units arrived this way, P&L on them per the decision.
- [ ] Replayed on a copy of the owner's book: neither "took more than the book held" line remains.

## Surfaces to check beyond the diff

`rust/crates/engine/src/matched` (beyond), `status.rs` `broker_failures`, `SPEC.md` §2 and §4.

## Right to refuse

If the broker's positions do not cover the days before a sale (not answered for a closed account), nothing is booked and the header keeps the sentence.

## Anti-stub self-check

Not built yet.

## Verification

Not built yet.

## Handoff

Blocked on the owner's decision above and the gate's verdict.
