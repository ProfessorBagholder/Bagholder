# bagholder-store

The SQLite store: `~/.bagholder/bagholder.db`, its schema and migrations, and
every read and write the app makes against it -- activities and the Wealthsimple
merge, CSV import, accounts, balances, margin, NAV, securities, the journal,
market data, orders and the feeds.

The DDL lives in `sql/schema_0.sql` and `sql/schema_1.sql` and is
`include_str!`d by `schema.rs`. `relabel::ensure` creates or migrates a
database and relabels option rows stored under Wealthsimple's own labels.

## Testing

    cargo test -p bagholder-store

## Demo book

`demo-book` writes a made-up book for the README screenshots:

    cargo run -p bagholder-store --bin demo-book -- --home /tmp/bh-demo        # a desktop data directory
    cargo run -p bagholder-store --bin demo-book -- --pull /tmp/bh-demo-phone  # last-pull.json + journal.json for the apps
