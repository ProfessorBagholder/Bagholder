# Plan: AI agents read the person's trade history through a read-only MCP connection

The owner's decision (2026-10-18): "building a read-only connection for other agents to access the trade history is the way to go." This plan is the gate for that feature (`CLAUDE.md`, the gate: a new feature).

## For the owner to decide

Nothing open. The owner decided (2026-10-18): **"same machine only for now."** Agents on the machine Bagholder runs on reach it over stdio, which needs no sign-in. Remote agents come with remote access, when the owner takes that up (`docs/decisions.md`).

## Scope

A command, `bagholder mcp`, that an MCP client starts on the person's machine. It speaks MCP over standard input and output, reads from the Bagholder server already running on that machine, and offers read-only tools:

- **trades**: the round trips, under the same filters the page has (account, symbol, dates, kind, result, grade, tag), each with its listing (`SYMBOL@VENUE`) and venue, its figures, and the journal (thesis, grade, tags), paged as the Trades list is;
- **trade**: one trade with its executions;
- **positions**: the holdings now, or as of a past day, with their figures;
- **journal lists**: the accounts, symbols, grades and tags in the book, so an agent can name a filter that exists.

**Out of scope, on purpose.** Anything that writes (the journal, the watchlist, orders); market data, news and filings (`disclosures-mcp` stays as it is, and the fold-in §14 describes is separate work); remote agents (decision 1); and any change to what the page shows. `SPEC.md` gains one section, on what the connection offers and how an agent reaches it, the way §Disclosures carries `disclosures-mcp`.

## The old app here

The old app had no agent access. Nothing is carried over.

## How the leading products do it

Each fact was read on 2026-10-18 from the source linked.

**The journals.**
- **TradeZella** offers an official MCP server for Claude, read-only by default. A read & write mode adds notes, tags and strategies, and neither mode can add, edit or delete a trade ([how to connect Claude to TradeZella MCP](https://help.tradezella.com/en/articles/17441265-how-to-connect-claude-to-tradezella-mcp)). This plan is that default: read only. Writing the journal is not asked for, and would be a plan of its own.
- **TradesViz** offers an MCP server and an API with scoped keys. Read, extended read, add/edit and delete are separate permissions, and it covers trades, executions, statistics, notes and tags ([TradesViz API and MCP guide](https://www.tradesviz.com/blog/tradesviz-api-mcp-guide/)). The data this plan exposes is that read set: trades, executions, the journal and holdings.
- **Tradervue** has a read-write REST API over trades, executions, journal, notes and comments, and no MCP server ([api-docs](https://github.com/tradervue/api-docs)).
- **TraderSync** and **Edgewonk** have neither. TraderSync's "API" means the broker sync keys ([support](https://tradersync.com/support/how-can-i-sync-my-trades/)), and Edgewonk exports to Excel, without notes ([export](https://edgewonk.zendesk.com/hc/en-us/articles/14639571465362-Export-journal-data)).

The journals that open to agents open read access first, and the one that also allows writes keeps trades out of it.

**The MCP specification** (revision 2026-07-28, the latest):
- **Two transports:** stdio, where "the client launches the MCP server as a subprocess" and they exchange newline-delimited JSON-RPC ([stdio](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio)); and Streamable HTTP. HTTP+SSE has been deprecated since 2025-03-26 ([transports](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports)).
- **Authorization is optional.** An HTTP server should follow OAuth 2.1: it must publish RFC 9728 protected resource metadata, and clients must send RFC 8707 resource indicators. A stdio server "SHOULD NOT follow this specification, and instead retrieve credentials from the environment" ([authorization](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization)). A local HTTP server must validate `Origin` and should bind to 127.0.0.1 ([Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)). This is why decision 1 matters: stdio needs no sign-in, and a network transport brings OAuth with it.
- **Tool annotations** (`readOnlyHint` and the rest) are hints, and "clients MUST consider tool annotations to be untrusted unless they come from trusted servers" ([tools](https://modelcontextprotocol.io/specification/2026-07-28/server/tools)). So read-only is enforced in the program, and the annotation only reports it.
- **Tools, not resources.** Tools are what the model calls. Resources are context the application chooses ([resources](https://modelcontextprotocol.io/specification/2026-07-28/server/resources)), and Anthropic's connector documentation covers tools only ([custom connectors](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)). Every read is a tool.

**The clients.**
- **Local:** Claude Code takes a stdio server with `claude mcp add <name> -- <command>` ([Claude Code MCP](https://code.claude.com/docs/en/mcp)). Claude Desktop takes one from `claude_desktop_config.json` ([connect local servers](https://modelcontextprotocol.io/docs/develop/connect-local-servers)).
- **Remote:** claude.ai's and Desktop's remote connectors connect "from Anthropic's cloud infrastructure", so the server "must be reachable over the public internet" ([custom connectors](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp)). For a self-hosted app that means exposing it to the internet, which decision 1 keeps out until remote access is taken up.

## Open questions

1. **Hand-rolled protocol, or the official Rust SDK (`rmcp`).**
   - *Objective:* one MCP implementation that stays current with the specification, under the rule "no hand-rolled server".
   - *Known:* `disclosures-mcp` hand-rolls newline-delimited JSON-RPC at protocol version 2024-11-05 (`rust/crates/market/src/bin/disclosures-mcp.rs:20`), and the specification is now at 2026-07-28 (see above). Whether `rmcp` speaks that revision is checked against its own release notes before it is added. The workspace already runs on tokio and serde, which the SDK uses.
   - *How it is settled:* add `rmcp` with its stdio transport only, after weighing its build time and binary size against the release archive (`CLAUDE.md`, *Do not*: weigh every dependency). Those are measured in the build, and the numbers go in the PR. If they are out of proportion, a small typed protocol module shared with `disclosures-mcp` is the alternative, and the PR says why.

## Approach

**Shape.** The same shape as `disclosures-mcp`, a stdio program the client starts, but with no data of its own. It is a client of the running server's typed interface (§12: one typed interface for every reader), on loopback. The figures it reports are therefore the page's, computed one way (§3, "many readers, one truth"). It reads them from what the server keeps current, and opening it makes the server do nothing new (§13). It lives in `rust/crates/server` as a subcommand of the `bagholder` binary (no fifth binary in the archive), with its tools in one module.

**Read-only, by type.**
- The adapter talks to the server through a read client, `ReadOnly`, whose methods are generated from the interface's `GET` routes alone (`web/src/lib/generated/routes.ts` is the page's copy of the same table). It has no method that can send a `POST`, so a tool that writes cannot be written without first changing that type.
- The adapter's one request function takes a `GET` route only (the route's type, not a string) and writes the method itself, so nothing the adapter holds can build another request.
- A test lists every tool and the route each one calls, and fails on any route that is not a `GET`. A source scan fails on any other HTTP client, or a method other than `GET`, anywhere in the adapter's module.
- Every tool is declared with the specification's read-only annotation (`readOnlyHint: true`), so a client can say so to the person. Clients treat annotations as hints, so the type is what enforces it.

**Which server.** The data folder's lock already says which server holds the folder and where it answers. `app::hold_home` holds `bagholder.lock` with an exclusive file lock for the server's life and writes `pid <n>` into it, then `pid <n>, http://127.0.0.1:<port>` once the server has bound (`note_home_port`). The adapter reads that file from `BAGHOLDER_HOME` (else the default data folder). It counts a server as running only while the file's lock is held: it tries the lock without waiting, and if it gets the lock, no server holds the folder and it releases the lock at once. That is a stronger test than a live pid, which can be reused. It takes the address from the file and confirms it with `/api/status` before it trusts it. A server still starting (a pid, no address yet) is answered as not ready, never guessed at. No change to `main.rs`, and nothing new written to the data folder. With no server running, each tool answers with that fact in words. The adapter never starts a server or opens the book itself, since two processes must never both write the book, and it never holds the folder's lock beyond that instant.

**What a tool answers.**
- Typed answers, the interface's own types: the same `Trade`, `Fill` and position rows the page receives, serialized as the wire serializes them, with amounts as exact decimals and never floats.
- Every list is paged, with the total, as the Trades list pages it. No cap of its own: an agent asks for the next page (`CLAUDE.md`: no arbitrary caps).
- The person's own text (thesis, tags) is the person's, and is marked as such, as §14 marks outside content. Instructions inside a thesis are data to the agent, never commands.

**SPEC.md.** A new section, *Agents*: what the connection offers, how to register it with Claude Code and Claude Desktop, that it reads only, and that it needs Bagholder running. `docs/architecture.md` §14's "not built" line is changed to say what is built.

**What stays the same.** The page, the server's routes, the book, `disclosures-mcp`.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace --locked` green, clippy clean.
- [ ] **Read-only by type:** `ReadOnly` has no method for a non-`GET` route. A planted tool that calls a `POST` route through any path the adapter has does not compile, or fails the route test.
- [ ] **One request path:** a source scan finds no other HTTP client and no method but `GET` in the adapter's module, and the request function's argument type admits `GET` routes only.
- [ ] **The right server:** with two servers running on two data folders, the adapter started with each folder reads that folder's book. A lock file whose lock no process holds is not trusted, even with an address in it. A server still starting (a pid, no address yet) is answered as not ready.
- [ ] **One truth:** for generated books (accounts, currencies, listings, open and closed trades, journal entries), each tool's answer equals the page's answer for the same filters, field for field.
- [ ] **Listing and venue:** each trade answer carries its listing key and venue, and a ticker on two venues gives two listings (generated).
- [ ] **Paging:** for generated sizes, paging through `trades` returns every trade exactly once, and the total matches the Trades list.
- [ ] **No server:** with no server running, each tool answers that Bagholder is not running, no process or file is created, and the folder's lock is left free.
- [ ] **A real client:** Claude Code registers the adapter (`claude mcp add bagholder -- bagholder mcp`), lists the tools with their read-only annotation, and answers a question about the made-up book. Recorded in the PR.
- [ ] **The container:** the adapter runs inside the image (`docker exec … bagholder mcp`) against the running server, driven by the same client.
- [ ] **Cost**, against §16: see *Cost* below. Numbers pasted in the PR.

### Cost

- **Nothing when unused.** The adapter is a process only while an agent has it open. The server gains no thread, timer or periodic read; `TIMED_WAITS` and the list of periodic reads are unchanged, and their tests hold that.
- **A tool call is a read the page already makes.** It is answered from the figures the server keeps current, with no recompute (§13). The capacity job gains one operation, *an agent's trades page* (`agent-trades-page`), measured at the owner's size and at four times under the growth rule.
  - **As measured, a page is not bounded by its size.** The Trades list's first page builds and sorts every trade in scope before it takes the page (`views::Trades::new`): allocations go 29,400 → 98,664 as the book goes 614 → 2,558 trades. That is the page's own read as much as the agent's. It is listed on `OVER_BUDGET` under #554, to be fixed as its own plan (the gate's starting point: a top-k selection bounded by the page).
- **Its wall time is reported, not budgeted.** No person waits on an agent's read the way they wait on an added trade, so it does not take a RAIL row.
- **Archive size:** the release archive before and after, given in the PR.
- **Idle hour:** unchanged by construction, since nothing runs while no agent is connected. The paired idle hour is still run once (the change touches the server).

## Surfaces to check beyond the diff

- `rust/crates/server/src/main.rs`: the subcommand dispatch only.
- `app::hold_home` and `note_home_port`: the lock file's contents the adapter reads.
- The route table the read client is generated from, and the test that holds it to the server's routes.
- `SPEC.md` (new section), `docs/architecture.md` §12 and §14, and `docs/decisions.md` (the owner's decision).
- `rust/Dockerfile` and the release workflow: the image and the archives carry the same `bagholder` binary, so nothing is added there; confirm it.
- `capacity.rs`: the new operation, and the test that holds its constants to §16's table.

## Right to refuse

If the read client cannot be made unable to write by its type (for instance, if the routes cannot be told apart by method where it is generated), stop and report before building. A read-only rule held by convention is not what the owner asked for. If the recorded client cannot connect to a subcommand of `bagholder`, report it with the client's error.

## Anti-stub self-check

Initialled at build:
- no tool nobody can call: the client listed and called all four;
- no field read from the server and dropped: the trades are held field for field to the page's;
- no path only a test switch reaches;
- the real client and the container actually run.

## Verification

Built on the home server; the PR carries the numbers and the transcripts.
- **The protocol.** `rmcp` 3.5.1 (2026-10-05) knows the 2026-07-28 revision. A client that still sends `initialize` is answered at 2025-11-25, the newest revision with that handshake, and a 2026-07-28 client is served without one.
- **Its weight:** 19 new crates, 58 CPU-seconds to build them in release; the `bagholder` binary 41.7 → 44.8 MB, 13.4 → 14.2 MB gzipped, as the release archive carries it.
- **Read-only.** The route test, the source scan, and the plants in the PR: a `GetRoute` made from a handler sent a body does not compile; a body-less `POST` fails the route test; a second request fails the scan.
- **A real client.** Claude Code, given the adapter by `--mcp-config` against a server on a copy of the made-up owner-size book, listed the four tools and answered from them.
- **The container.** The image built from this branch, the adapter run with `docker exec … bagholder mcp` against the server in it.

## Handoff

Built in the PR that carries this plan. The cost of the Trades view's first page is #554.

**Nothing left running.**
