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
- **Checked once, 2026-09-24** (brief 01 §3.2): an agent that had not read the engine re-derived every case in `positions_and_income.json` and `rates.json` and the first case of every other file from `SPEC.md` and the design. Every figure agreed. It found one thing the spec does not define: the trade flags beyond `reward` and `basis-unknown` (`docs/architecture.md` §18).
- An amount is compared at the places it is written to (`"185.00"` is the figure rounded half to even to two places).
