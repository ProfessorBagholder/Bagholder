# bagholder-model

The derived trading model in Rust, ported from `model.py`.

## What is here

The pipeline `model.py` documents, end to end:

    activities -> normalize -> match_fifo -> apply_fx -> trades
                                                      -> positions
                                                      -> cashflow
    nav_history -> equity series, yearly returns, drawdown
    build_view(filters) -> the KPIs, tables and tiles

`markets_view` is not ported yet: the Markets tab needs `instruments.py`,
`news.py` and the universes, and nothing in `build_view`'s other figures
depends on it.

## Checking it against the Python model

Fidelity is not assumed anywhere. Build the tools, then run whichever check
covers what changed:

    cargo build

    python3 crates/model/casetest.py   # the 33 shared cases, through build_view
    python3 crates/model/booktest.py   # build_base's pieces, field by field
    python3 crates/model/fifotest.py   # match_fifo on every case
    python3 crates/model/difftest.py   # normalization and the symbol readers
    python3 crates/model/fuzz.py 2000  # random books neither set covers

`casetest.py` is the one that matters: it runs the same `tests/cases` files
`tests/test_cases.py` runs, against the same `expect`, so the Rust model and
the Python one cannot disagree without a failing test.

## Two things that had to be chosen deliberately

`serde_json` carries `float_roundtrip`. Its default float parser reads
`120.00000000000001` as `120.0`, and a case carries that value.

The clock reads `America/Edmonton` from the system's tzdata rather than a
snapshot compiled in, because Python's `zoneinfo` reads the same files.
Alberta moves to permanent Central Standard Time during 2026, so a bundled
database one version behind puts every winter fill in the wrong hour.
