# Plan: an agent reads everything the page shows, through the one MCP server

## For the owner to decide

Nothing open: the owner's rule is in `docs/decisions.md` (2026-11-02).

## Scope

**In it:**
- Every read Bagholder's page makes gets an agent tool on the agents' MCP server (and its JSON API twin), read-only, answering exactly what the page gets.
- A test holds that a page read without an agent tool fails the build.
- `disclosures-mcp`, the first cut of this server, is retired.

**Out of it:**
- Anything that writes: orders, the journal, settings. Agents stay read-only, as they are today.
- The agents' own administration: the panel's log, keys and sign-ins.

## What the page reads that agents can't, today

Read from the code on 2026-11-02: the page's `GET` routes and view documents against `agents/tools.rs`.

| What the person sees | Page reads | Agent tool today |
|---|---|---|
| Trades, a trade, holdings, Dashboard, Cashflow income, Exposure, Markets tiles and watchlist, Book lists, spending transactions | views `trades`, `trade:`, `positions`, `dashboard`, `cashflow`, `exposure`, `markets`, `book`; `/api/transactions` | yes (10 tools) |
| News headlines, and a listing's news | view `headlines`; `/api/news/symbol` | **none** |
| Disclosures and filings, their feed, and their summaries | `/api/filings`, `/api/filings/feed`, `/api/filings/enrich` | **none** |
| Short interest, and its feed | `/api/shorts`, `/api/shorts/feed` | **none** |
| The Markets heatmap | view `heatmap` | **none** |
| Fear & Greed | `/api/fear` | **none** |
| A listing's price history (the charts) | `/api/history` | **none** |
| Quotes, listing details and symbol search | `/api/symbols/quote`, `/api/listing`, `/api/listings`, `/api/symbols/search` | **none** |
| Orders (open and past) and an order's quote | `/api/orders`, `/api/order/quote` | **none** (read only) |
| Notifications | `/api/notifications` | **none** |
| Several trades' details at once | `/api/figures/details` | the `trade` tool covers one |
| Connection and sync state | `/api/status` | **none** (what the header shows: last sync, errors) |

## How the leading products do it

Read 2026-11-02.
- **The Model Context Protocol's tools specification** describes a server's tools with annotations, `readOnlyHint` among them, for a client to tell reads from writes (modelcontextprotocol.io, specification, tools). Bagholder's ten tools already carry it; the new ones do too.
- **One application, one server.** Coinbase's MCP server for agents, in use in this workspace, serves one account's balances, portfolios, orders and products from a single server, with writes only on the portfolios the person authorized (its own server instructions). The application's data isn't split across servers by kind.

## Approach

- **One mechanism.** `agents::tools::ReadOnly` already answers a tool by asking the page's own view (`r.view(...)`), so a tool can never disagree with the screen. Each new tool is that: a view document, or the same handler a page route uses, called through `ReadOnly`, never a second query.
- **New tools:**
  - **News:** headlines, a listing's news.
  - **Disclosures:** an instrument's filings from every source (SEDAR+, EDGAR) merged as the page shows them, the feed, a filing's summary, and a filing's document link.
  - **Short interest:** a listing's, and the feed.
  - **Heatmap.**
  - **Fear & Greed.**
  - **Price history:** a listing's bars for a range.
  - **Quotes and listings:** a quote, a listing's details, symbol search.
  - **Orders:** open and past, read only.
  - **Notifications:** read only; reading doesn't mark them read.
  - **Status:** what the header says.

  Each takes the arguments the page passes (filters, listing, range, paging), carries `read_only_hint = true`, and pages as the existing tools do.
- **The test that holds it:** the page's read routes and view keys are listed from the router and `views.rs`. Each must be named in a table of agent tools or in a short, reasoned list of what is not data: the agents' own panel, the stream. A new page read with neither fails.
- **Retiring `disclosures-mcp`:** the binary (`market/src/bin/disclosures-mcp.rs`), `rust/mcp/manifest.json`, its lines in `rust/Dockerfile`, `.github/workflows/release.yml`, `README.md`, `DISCLOSURES.md`, `SPEC.md`, `CLAUDE.md` and `docs/architecture.md`. Its one capability the page lacks is downloading a filing's document; that goes into the Disclosures tool as the document's link, which the page already shows. The `sedar` debugging program stays: it's a developer's tool, not an agent surface.
- **Agents' tool descriptions** say what each reads in the page's own words, as the existing ten do.

## Acceptance criteria

- [ ] Every row marked **none** above has a tool, each answering the same document the page gets for the same arguments. Held by a server test per tool that compares the tool's answer with the page route's.
- [ ] The coverage test: every page read is an agent tool or on the reasoned list. Plant: a page route added without a tool fails it.
- [ ] Every tool carries `read_only_hint = true`, and no tool reaches a handler that writes. Held by a test over the tool list.
- [ ] `disclosures-mcp` is gone from the workspace, the image, the release archives and every document. The release's archive list and the Dockerfile test are updated.
- [ ] An agent, through the stand-in MCP client, reads a holding's news, filings and short interest on the made-up book.
- [ ] Capacity: the new tools' reads are the page's own, so no new cost. The capacity test still passes.
