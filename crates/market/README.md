# bagholder-market

The public market sources in Rust, ported from `market.py`.

Ported so far: the HTTP client, every parser, and the top-ups for USD/CAD, the
S&P 500 and the two TMX indices. The per-symbol quote and history fetchers are
still Python's.

    cargo build -p bagholder-market
    python3 crates/market/parsetest.py     # every parser, on fixtures
    python3 crates/market/refreshtest.py   # both sides fetch live, stores compared

## Why the client is hand-written over OpenSSL

Two of these hosts will not talk to an off-the-shelf client.

FRED answers an OpenSSL handshake and stonewalls both rustls and macOS
SecureTransport — the same kind of TLS-keyed gate `curl_cffi` exists for on the
Python side. Python reaches these sources through its `ssl` module, which is
OpenSSL, so this does too.

FRED also never replies to a keep-alive request that asks for gzip, and answers
the same request at once when it asks for `identity`. Python's `http.client`
sends `Accept-Encoding: identity` by default, which is why it never hit this;
the Rust client now sends the same.
