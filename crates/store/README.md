# bagholder-store

The SQLite store in Rust, ported from `store.py`. It reads and writes the same
`~/.bagholder/bagholder.db` the Python app does.

Ported so far: the schema, every migration, and `ensure()`.

The DDL is not retyped. `sql/schema_0.sql` and `sql/schema_1.sql` are the exact
text `store._init_schema` executes, extracted from it and `include_str!`d, so
the two cannot drift.

    cargo build -p bagholder-store
    python3 crates/store/ensuretest.py

`ensuretest.py` writes a database, fills it with raw Wealthsimple option rows
under the broker's own labels, copies it, runs each implementation's `ensure()`
on its own copy and compares every table row for row -- so the relabelling and
the one-shot unit-price scaling cannot differ.
