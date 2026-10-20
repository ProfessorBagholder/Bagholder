# bagholder-store

The SQLite store: `~/.bagholder-rust/bagholder.db`, its schema and migrations, and
every read and write the app makes against it -- activities and the Wealthsimple
merge, CSV import, accounts, balances, margin, NAV, securities, the journal,
market data, orders and the feeds.

The DDL lives in `sql/schema_0.sql` and `sql/schema_1.sql` and is
`include_str!`d by `schema.rs`. `relabel::ensure` creates or migrates a
database and relabels option rows stored under Wealthsimple's own labels.

## Testing

    cargo test -p bagholder-store

## Demo book

The made-up book for the browser tests and the README screenshots is written by
the server straight into a book, never into this store's database:

    cargo run -p bagholder-server --bin bagholder --features demo -- demo /tmp/bh-demo   # a data folder holding no book yet
