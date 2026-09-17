# Shared model cases

One file per case in `cases/`. Every implementation of the Bagholder model (Python in `python/model.py`, Rust in `rust/crates/model`, Swift in `ios/Bagholder/Model.swift`, Kotlin in `android/model`) reads these files in its own test suite, runs the rows through its own model, and compares with `expect`. A change to a rule that is not made in every implementation fails that implementation's tests.

```
{
  "today":    "YYYY-MM-DD",          the day the model is built for
  "snapshot": {"activities": [...], ...},   Wealthsimple activity rows as the sync stores them
  "market":   {"fx": {...}, "benchmark": {...}, "distributions": {...}},
  "filters":  {},                    the filter set applied to the view
  "expect":   {"kpi": ..., "trades": [...], "positions": [...], "cashflowHoldings": [...], "cashflowTiles": [...]}
}
```

`expect` holds only the fields listed in `python/tests/make_cases.py` (`TRADE_KEYS`, `KPI_KEYS`, `POSITION_KEYS`, `HOLDING_KEYS`, `TILE_KEYS`; the cashflow lists appear when the case has a dividend row), floats rounded to six places, lists sorted as the generator sorts them. The meaning of every field is in `SPEC.md`.

The Python model is the reference. After an intended model change, made in both desktop implementations: `python3 python/tests/make_cases.py`, review the diff of `cases/`, commit both. `python/tests/test_cases.py` and `cargo test -p bagholder-model --test cases` (in `rust/`) fail until that is done, on purpose. To add a case, add it to `CASES` in `make_cases.py` and regenerate.
