# bagholder-market

The public market sources: the HTTP client, quotes and price history (Yahoo,
TMX Money, Cboe Canada, Coinbase), exchange rates and benchmarks (Bank of
Canada, FRED), news, short-selling reports, fund exposure, market universes,
the fear and greed indices, and regulatory filings (EDGAR, SEDAR+) with their
text and summaries.

    cargo build -p bagholder-market
    cargo test -p bagholder-market     # every parser on fixtures; no network

## Why the client is hand-written over OpenSSL

Two of these hosts will not talk to an off-the-shelf client.

FRED answers an OpenSSL handshake and stonewalls both rustls and macOS
SecureTransport, so the client speaks through OpenSSL. The hosts that key on a
browser's handshake (Yahoo's statistics, SEDAR+) go through the
`bagholder-browser` helper instead.

FRED also never replies to a keep-alive request that asks for gzip, and answers
the same request at once when it asks for `identity`, so the client sends
`Accept-Encoding: identity` unless a caller asks otherwise.
