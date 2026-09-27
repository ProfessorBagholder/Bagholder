# The old model's cases

**Frozen with the old model.** These cases were generated from the old Python model and record what the old app computes, mistakes included (`docs/old-app-mistakes.md`); they are not a statement of what is right. They are kept as they are for the implementations that still read them. The new engine's cases are in `rust/crates/engine/tests/cases`, written from `SPEC.md`.

One file per case in `cases/`. The implementations of the old model (Rust in `rust/crates/model`, Swift in `ios/Bagholder/Model.swift`, Kotlin in `android/model`) read these files in their own test suites, run the rows through their own model, and compare with `expect`.

```
{
  "today":    "YYYY-MM-DD",          the day the model is built for
  "snapshot": {"activities": [...], ...},   Wealthsimple activity rows as the sync stores them
  "market":   {"fx": {...}, "benchmark": {...}, "distributions": {...}},
  "filters":  {},                    the filter set applied to the view
  "expect":   {"kpi": ..., "trades": [...], "positions": [...], "cashflowHoldings": [...], "cashflowTiles": [...]}
}
```

`expect` holds a chosen set of fields of each figure (the cashflow lists appear when the case has a dividend row), floats rounded to six places, lists sorted. The meaning of every field is in `SPEC.md`. `cargo test -p bagholder-model --test cases` (in `rust/`) runs them.

`wire/` holds what the old model sent the page for each case, whole: `cargo test -p bagholder-model --test wire` holds the Rust model to it, and `bagholder-diff`'s tests patch each view into every other. `fixtures/` holds recorded SEDAR+ pages the market crate's tests read.
