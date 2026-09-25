# The engine's cases

Each file here is a list of cases the engine is held to (`../cases.rs` runs them all with `cargo test -p bagholder-engine --test cases`). A case is a small book, what the market and the facts say, and the figures `SPEC.md` requires of it:

```
{
  "name":    what the case shows, as a sentence
  "spec":    the SPEC.md section (or design section) the figures come from
  "working": the figures worked out by hand from that definition, step by step
  "today", "now", "rates", "covered", "series", "closes", "quotes", "transactions", ...: the inputs
  "expect":  the figures, as exact decimal text, or {"gaps": [...]} where the spec says the figure waits
}
```

- **Written from `SPEC.md`, never generated from any implementation.** No expected figure is produced by running this engine or any other; the working beside it shows how it follows from the definition.
- **New expectations come from an agent that has not read the engine.** It is given `SPEC.md`, this format and the inputs, and told not to read `rust/crates/engine`; the session building the engine only implements. A disagreement between the two is settled against `SPEC.md`; a real ambiguity in `SPEC.md` goes to the owner as one question.
- **Checked once, 2026-09-24** (brief 01 §3.2): an agent that had not read the engine re-derived every case in `positions_and_income.json` and `rates.json` and the first case of every other file from `SPEC.md` and the design. Every figure agreed. It found one thing the spec does not define: the trade flags beyond `reward` and `basis-unknown` (`docs/architecture.md` §18). **Checked whole, 2026-09-24** (brief 04 §8): every case of every file, blind copies without their expectations, 704 figures. 670 agreed. Of the rest, three committed figures were wrong: `fills.json` left converted cash out of the price check (now compared within the rounding of the cash and the stated rate), `positions_and_income.json` wrote a yield waiting on its schedule as `null` (now its gap, and the runner reads a ratio's gaps), and `coins_and_transfers.json` makes a trade of a round trip still open, which waits on the plan `docs/plans/trade-open-to-flat.md`. Four blind figures were wrong; 55 were points the specs leave open (the format of `null`, `needs_trade` and `opened_by` here; the trade flags; a short's Market sign; cash in lieu per unit; rounding of the price check).
- An amount is compared at the places it is written to (`"185.00"` is the figure rounded half to even to two places).
- **Open trades, 2026-09-24** (`docs/plans/trade-open-to-flat.md`): an agent that had not read the engine re-derived the eleven cases the change moved and wrote `open_trades.json` from `SPEC.md`. Two readings `SPEC.md` left open were settled and written into it or the case's working: an open trade with no sale has realized 0.00 (not a gap); every child of a spin-off is opened by the event its adjustment applies to. Still undefined, and not asserted by any case: a roll chain's Qty and Entry across its contracts.
- **A distribution's form, 2026-09-25** (`docs/plans/stage-3c-switch.md`, §3a): an agent that had not read the engine derived every figure of `distribution_form.json` from `SPEC.md` §2 Distribution rate; the engine agreed with each. Open in the spec and written as the agent read it: `next_ex` names the latest distribution when none is left to pay, whether it was paid in cash or in units.
