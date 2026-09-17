# Shared model cases

One file per case in `cases/`. Every implementation of the Bagholder model (Go in `internal/model`, Swift in `ios/Bagholder/Model.swift`, Kotlin in `android/model`) reads these files in its own test suite, runs the rows through its own model, and compares with `expect`. A change to a rule that is not made in every implementation fails that implementation's tests.

```
{
  "today":    "YYYY-MM-DD",          the day the model is built for
  "snapshot": {"activities": [...], ...},   Wealthsimple activity rows as the sync stores them
  "market":   {"fx": {...}, "benchmark": {...}, "distributions": {...}, "quotes": {...}},
  "filters":  {},                    the filter set applied to the view
  "journal":  {},                    journal entries by trade id
  "expect":   {"kpi": ..., "trades": [...], "positions": [...], "cashflowHoldings": [...], "cashflowTiles": [...]}
}
```

`expect` holds only the fields listed in `internal/cases/cases.go` (`TradeKeys`, `KPIKeys`, `PositionKeys`, `HoldingKeys`, `TileKeys`; the cashflow lists appear when the case has a dividend row), floats rounded to six places, lists sorted as the generator sorts them. The meaning of every field is in `SPEC.md`.

The Go model is the reference. After an intended model change: `go run ./tests/makecases`, review the diff of `cases/`, commit both. `go test ./internal/model/` fails until that is done, on purpose. The generator rewrites only `expect`; the inputs of a case are kept byte for byte. To add a case, write a new file under `cases/` with its `today`, `snapshot`, `market`, `filters` and `journal` (an empty `expect` is fine) and regenerate.
