# bagholder-server

The server in Rust, ported from `bagholder.py`.

Ported so far: the gate, the page and its assets, `/api/status`, and
`/api/model` with the base cache behind it. The Wealthsimple client, the sync,
the order routes and everything else that writes are still Python's.

    cargo build --release -p bagholder-server
    BAGHOLDER_HOME=/tmp/scratch BAGHOLDER_PORT=8799 ./target/release/bagholder

Run it against a copy of a database, never the live one, and never on 8765
while the Python app is using it.

`versions()` is the one place the two cannot agree by value. Python's
fingerprint embeds `hash(str)`, which is salted per process, so the number it
produces is only ever comparable to another taken by the same run. The Rust
side uses a stable hash instead: same behaviour, different digits.
