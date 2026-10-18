# Brief 54: Verdict on plan #535 (agents read the trade history): Go with changes

## Verdict

**Go with two changes.** The plan is the design the category points to, and the right one for this app:
- a stdio MCP program the client starts, holding no data of its own;
- reading the running server's typed interface, so its figures are the page's;
- read-only enforced by the adapter's type, not by the annotation;
- answers in typed form, every list paged;
- tests over generated books.

Its research is sourced and dated: TradeZella's read-only default, TradesViz's scoped reads, and the MCP specification's transports, authorization and tool annotations.

The owner's decision (local agents only, or remote too) went to the owner from the reviewer, in plain words. The owner's answer (2026-10-18): **"same machine only for now."** Record it in `docs/decisions.md` in this PR, in those words, and replace the plan's *For the owner to decide* with the decision as made. Remote agents come with remote access, when the owner takes that up.

## Required changes

1. **Read-only rests on the adapter's type alone, and the plan says nothing about the server's write check.**
   - Remove the bullet that begins "The server's own write check is no barrier here", and the words "and the token rule" in the annotation bullet.
   - Remove "the Host/Origin check and the write token" from *What stays the same*.

   The owner has closed that subject. The adapter's type, the route test and the source scan carry the rule whole, and need nothing beside them.

2. **No port file. The data folder's lock already says which server holds the folder and where it answers.** `app::hold_home` holds `bagholder.lock` with an exclusive file lock for the server's life, and writes `pid <n>` into it. Once the server has bound, it writes `pid <n>, http://127.0.0.1:<port>` (`app.rs`, the address written beside the pid). The adapter:
   - reads that file from `BAGHOLDER_HOME`, else the default data folder;
   - takes a server as running only while the file's lock is held. It tries the lock without waiting: if it gets it, no server holds the folder, and it releases the lock at once. That is a stronger test than a live pid, which can be reused;
   - takes the address from the file, then confirms with `/api/status` as planned.

   No change to `main.rs`, and nothing new written to the data folder.
   *Tests* (replacing *The right server*):
   - two servers on two folders, each adapter reads its own book;
   - a lock file whose lock no process holds is not trusted, even with an address in it;
   - a server still starting (pid but no address yet) is answered as not ready, never guessed at.

   The adapter must never hold the lock itself beyond that instant, so the "no server" criterion gains: the adapter leaves the folder's lock free.

Everything else stands as written, including the open question on `rmcp`, settled by measurement in the PR.

## For the CTO

Build after the bugs now on the list, unless the owner says otherwise (`docs/decisions.md`, 2026-10-04: no new features until the bugs are fixed).
