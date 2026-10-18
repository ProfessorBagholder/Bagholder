# Plan: AI agents read the person's trade history through a read-only MCP connection

The owner's decision (2026-10-18): "building a read-only connection for other agents to access the trade history is the way to go." This plan is the gate for that feature (`CLAUDE.md`, the gate: a new feature).

## For the owner to decide

1. **Which agents it reaches now: the ones on this machine only, or remote ones too.**
   - *Why it matters.* An agent on the same machine as Bagholder (Claude Code, Claude Desktop, any MCP client the person runs) starts the connection itself and needs no sign-in: the machine's own account is the boundary, as it is for the page today. An agent elsewhere (another computer, or a hosted assistant such as claude.ai's connectors) has to reach Bagholder over the network. The MCP specification then calls for an OAuth sign-in, and a hosted assistant connects from its provider's cloud, so Bagholder would have to be reachable from the internet (see *How the leading products do it*). That is the remote access with a sign-in that you put off on 2026-10-04 ("not now"), and the "no tokens with access classes" decision of 2026-09-25 rules out a cheaper key.
   - *Recommendation.* Local agents only now, over the transport every MCP client supports for a program on the same machine (stdio). Remote agents come with remote access, when you take that up, as one more client of the same sign-in. If you want remote agents now, this plan has to take on remote access with its sign-in first, which is a plan of its own.

Nothing else is open: the rest follows from the decision, `docs/architecture.md` §12 to §16 and the MCP specification.

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
- The server's own write check is no barrier here, and the plan does not lean on it. It tells the app's page from another website (a header a browser cannot send cross-site, §12). Any program on the machine can send that header, and the token it stands for is the same on every install (the issue §12 names). Read-only is held by the adapter's type, and by nothing else.
- Every tool is declared with the specification's read-only annotation (`readOnlyHint: true`), so a client can say so to the person. Clients treat annotations as hints, so the type and the token rule are what enforce it.

**Which server.** The server takes the first free port of its list (`PORTS`, `main.rs`) unless `BAGHOLDER_PORT` names one, so the adapter cannot know it in advance. The server therefore writes the port it bound, with its own process id, to a file in its data folder once it listens, and removes it on a clean stop. The adapter reads that file from `BAGHOLDER_HOME` (else the default data folder), and checks the process is alive and that `/api/status` answers from that folder before it trusts it. It connects to `127.0.0.1` only. With no server running, each tool answers with that fact in words, and the adapter never starts a server or opens the book itself. Two processes must never both write the book, and the agent's read must never hold a lock the running app needs.

**What a tool answers.**
- Typed answers, the interface's own types: the same `Trade`, `Fill` and position rows the page receives, serialized as the wire serializes them, with amounts as exact decimals and never floats.
- Every list is paged, with the total, as the Trades list pages it. No cap of its own: an agent asks for the next page (`CLAUDE.md`: no arbitrary caps).
- The person's own text (thesis, tags) is the person's, and is marked as such, as §14 marks outside content. Instructions inside a thesis are data to the agent, never commands.

**SPEC.md.** A new section, *Agents*: what the connection offers, how to register it with Claude Code and Claude Desktop, that it reads only, and that it needs Bagholder running. `docs/architecture.md` §14's "not built" line is changed to say what is built.

**What stays the same.** The page, the server's routes, the book, `disclosures-mcp`, the Host/Origin check and the write token.

## Acceptance criteria

- [ ] `RUSTFLAGS="-D warnings" cargo test --workspace --locked` green, clippy clean.
- [ ] **Read-only by type:** `ReadOnly` has no method for a non-`GET` route. A planted tool that calls a `POST` route through any path the adapter has does not compile, or fails the route test.
- [ ] **One request path:** a source scan finds no other HTTP client and no method but `GET` in the adapter's module, and the request function's argument type admits `GET` routes only.
- [ ] **The right server:** with two servers running on two data folders, the adapter started with each folder reads that folder's book; a port file left by a server that died is not trusted.
- [ ] **One truth:** for generated books (accounts, currencies, listings, open and closed trades, journal entries), each tool's answer equals the page's answer for the same filters, field for field.
- [ ] **Listing and venue:** each trade answer carries its listing key and venue, and a ticker on two venues gives two listings (generated).
- [ ] **Paging:** for generated sizes, paging through `trades` returns every trade exactly once, and the total matches the Trades list.
- [ ] **No server:** with no server running, each tool answers that Bagholder is not running, and no process, file or lock is created.
- [ ] **A real client:** Claude Code registers the adapter (`claude mcp add bagholder -- bagholder mcp`), lists the tools with their read-only annotation, and answers a question about the made-up book. Recorded in the PR.
- [ ] **The container:** the adapter runs inside the image (`docker exec … bagholder mcp`) against the running server, driven by the same client.
- [ ] **Cost**, against §16: see *Cost* below. Numbers pasted in the PR.

### Cost

- **Nothing when unused.** The adapter is a process only while an agent has it open. The server gains no thread, timer or periodic read; `TIMED_WAITS` and the list of periodic reads are unchanged, and their tests hold that.
- **A tool call is a read the page already makes.** It is answered from the figures the server keeps current, with no recompute (§13). The capacity job gains one operation, *an agent's trades page*: measured at the owner's size and at four times, under the growth rule (one page's work under twice at four times the size, since a page is bounded by its size and not by the book).
- **Its wall time is reported, not budgeted.** No person waits on an agent's read the way they wait on an added trade, so it does not take a RAIL row.
- **Archive size:** the release archive before and after, given in the PR.
- **Idle hour:** unchanged by construction, since nothing runs while no agent is connected. The paired idle hour is still run once (the change touches the server).

## Surfaces to check beyond the diff

- `rust/crates/server/src/main.rs`: the subcommand dispatch, and the port file written once the server listens and removed on a clean stop.
- The route table the read client is generated from, and the test that holds it to the server's routes.
- `SPEC.md` (new section), `docs/architecture.md` §12 and §14, and `docs/decisions.md` (the owner's decision).
- `rust/Dockerfile` and the release workflow: the image and the archives carry the same `bagholder` binary, so nothing is added there; confirm it.
- `capacity.rs`: the new operation, and the test that holds its constants to §16's table.

## Right to refuse

If the read client cannot be made unable to write by its type (for instance, if the routes cannot be told apart by method where it is generated), stop and report before building. A read-only rule held by convention is not what the owner asked for. If the recorded client cannot connect to a subcommand of `bagholder`, report it with the client's error.

## Anti-stub self-check

To initial at the end: no tool nobody can call; no field read from the server and dropped; no path only a test switch reaches; the real client actually run against the made-up book and the container, not asserted.

## Verification

*Filled when built.*

## Handoff

*Filled when built.*

**Nothing left running.** *Filled when built.*
