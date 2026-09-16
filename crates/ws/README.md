# bagholder-ws

The Wealthsimple client, ported from `bagholder.py`.

Ported so far: the GraphQL operations and the activity mapper. The session,
the token refresh and the sync itself are next; the order routes after that.

The seventeen operations in `graphql/` are the exact text `bagholder.QUERIES`
holds, extracted from it rather than retyped, and `qcheck` compares them
character for character. They were recovered from the public web bundle.

    cargo build -p bagholder-ws
    python3 crates/ws/maptest.py
