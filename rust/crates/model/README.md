# bagholder-model

The derived trading model: every figure the page and the apps show, computed
from the stored activities, market data and filters.

## What is here

    activities -> normalize -> match_fifo -> apply_fx -> trades
                                                      -> positions
                                                      -> cashflow
    nav_history -> equity series, yearly returns, drawdown
    build_view(filters) -> the KPIs, tables and tiles
    markets_view -> the Markets tab

## Testing

    cargo test -p bagholder-model

`tests/cases.rs` runs the shared model cases in `tests/cases` at the
repository root through `build_view` and compares each against its `expect`;
the iOS and Android models run the same files, so no implementation can
disagree with another without a failing test. `tests/model.rs` and
`tests/instruments.rs` cover the pieces the cases do not reach.

## Two things chosen deliberately

`serde_json` carries `float_roundtrip`. Its default float parser reads
`120.00000000000001` as `120.0`, and a case carries that value.

The clock reads `America/Edmonton` from the system's tzdata rather than a
snapshot compiled in. Alberta moves to permanent Central Standard Time during
2026, so a bundled database one version behind puts every winter fill in the
wrong hour.
