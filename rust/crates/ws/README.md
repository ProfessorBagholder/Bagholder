# bagholder-ws

The Wealthsimple client: the session and token refresh, the GraphQL operations,
the account, activity, balance, margin, NAV and security fetches, the activity
mapper, and one sync pull.

The operations in `graphql/` are the exact text Wealthsimple's web client
sends, recovered from its public web bundle and included rather than retyped.

## Tests

    cargo test -p bagholder-ws

`tests/mapping.rs` covers the mapper, sync bounds and NAV helpers with no
network; `tests/http.rs` runs the client against a stand-in server on
127.0.0.1.
