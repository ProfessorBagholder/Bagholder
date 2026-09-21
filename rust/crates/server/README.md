# bagholder-server

The Bagholder desktop app: the HTTP server on 127.0.0.1 that serves the page
and its API, the background loops (sync, quotes, market data, news, filings,
short selling, orders and brackets), notifications, the Wealthsimple sign-in
window and the in-app updater.

    cargo build --release -p bagholder-server
    BAGHOLDER_NO_BROWSER=1 BAGHOLDER_DRY_ORDERS=1 BAGHOLDER_HOME=/tmp/bh-scratch BAGHOLDER_PORT=8799 ./target/release/bagholder

Run it against a copy of a database, never the live one, and never on 8765.

## Tests

    cargo test -p bagholder-server

Wealthsimple is always a fake in the tests; nothing reaches the network and no
order is placed.

`versions()` fingerprints what the derived model reads. It is only compared
with another fingerprint taken by the same run, and uses a stable FNV-1a hash
of the stored text.
