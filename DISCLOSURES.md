# Disclosures

An instrument's regulatory filings, gathered from every source that covers it,
normalized to one shape, merged newest first. Two dimensions run through every
item and both are open sets — a new source or a new category is additive:

- **Source** — the regulator's filing system. Today: **SEDAR+** (Canadian company
  filings) and **SEC EDGAR** (US filings, including US insider). Planned: **SEDI**
  (Canadian insider), then other markets (UK, Australia, Hong Kong, Japan) as
  Bagholder adds brokerages or Wealthsimple adds listings.
- **Category** — the kind of disclosure, the same words across sources: Financials,
  Material events, Governance, Offerings, Insider & ownership, News releases, Other.

A cross-listed issuer (Shopify, say) draws from more than one source at once; the
list interleaves them by date. Third-party media coverage is **not** here — it
stays in the News card. This is the issuer's own filed record.

## How it fits together

- `disclosures.py` — the pipeline: the shared vocabulary (categories), the provider
  registry, and the merge. It dispatches by the instrument's market, calls each
  covering provider, and returns one sorted list plus a per-source status.
- `sedar.py` — the SEDAR+ provider. Reaches the site through `curl_cffi` (a browser
  TLS fingerprint, the only client its Radware bot gate admits); resolves an issuer
  to its profile, lists filings, downloads a PDF. Needs the `curl_cffi` dependency.
- `edgar.py` — the SEC provider. A documented JSON API, no gate, no key, standard
  library only, so US filings work even without `curl_cffi`. Set `BAGHOLDER_SEC_UA`
  to your own contact (SEC asks callers to identify themselves).

An item is `{id, source, category, date, dateText, type, title, size, url}`, with
`id` prefixed by source (`sedar:…`, `sec:…`) so a download routes back to the right
provider. It is cached per symbol and refreshed when asked for and older than a day.

## Three ways to reach it

### 1. The running app's endpoint

While `bagholder.py` is running (default `http://127.0.0.1:8765`):

    GET /api/filings?symbol=SHOP            merged list from cache
    GET /api/filings?symbol=SHOP&refresh=1  fetch first, then return
    GET /api/filings?symbol=SHOP&name=Shopify%20Inc.   seed the issuer lookup
    GET /api/filings/doc?symbol=SHOP&id=<item id>       the document itself

The payload carries the merged `filings`, a per-source `sources` status
(available / matched), the `categories` vocabulary, the SEDAR+ `profileNo`, and
`fetchedAt`. A filing never changes once filed.

### 2. The MCP server (for Claude Desktop, Claude Code, any MCP client)

`disclosures_mcp.py` exposes `disclosures_list`, `disclosures_document`, and
`sedar_resolve_profile` over stdio.

Claude Code:

    claude mcp add disclosures -- python3 /full/path/to/disclosures_mcp.py

Claude Desktop (Settings > Developer > Edit Config), under `mcpServers`:

    "disclosures": { "command": "python3", "args": ["/full/path/to/disclosures_mcp.py"] }

To install it as a one-click Claude Desktop extension, pack `mcp/manifest.json`
with the `mcpb` CLI (`npx @anthropic-ai/mcpb pack`) and open the resulting
`.mcpb` file.

### 3. The SEDAR+ command line

`sedar.py` is a SEDAR-only utility for a shell or Claude Code:

    python3 sedar.py resolve "Shopify"           SEDAR+ profiles matching an issuer
    python3 sedar.py filings "Shopify" 50         an issuer's SEDAR+ filings (JSON)
    python3 sedar.py newest 30                    newest SEDAR+ filings, any issuer
    python3 sedar.py get <profileNo> <id> out.pdf download one SEDAR+ document

## What each filing is about

A filing list tells you the type, date and source, not what a document contains.
Two enrichments (`enrich.py`) fill that in, both local. When a filing list is shown
the rows are enriched on their own, top of the list first, one document at a time at
SEDAR+'s pace (the issuer's document scope is cached for the run, so the list is read
in one walk, not one per row), and every result is stored so a later visit is instant:

- **Title** — what the filing is, one short phrase, on every row. The document's own
  title when it exposes one (a SEDAR+ PDF's metadata: "CHARBONE - Closing 2nd
  Drawdown"), otherwise one the local model derives from the filing's substance. For a
  SEC filing whose primary document is a cover form (a 6-K, an 8-K), the substance is
  in its exhibits; `edgar.content` resolves to the real content document so the title
  and summary describe it, not the boilerplate. A structured filing (a Schedule
  13G/13D) is parsed exactly from its XML by `edgar.enrichment` — the stake and the
  holder — with no model at all.
- **Summary** — one plain sentence of what the filing announces, from a language
  model running **locally**, so nothing leaves the machine and there is no key or
  bill. It is **automatic**: the first time you open a filing, the app looks for a
  local model server you already run (Ollama, or anything at `BAGHOLDER_LLM_URL`)
  and uses it; if there is none, it downloads a small self-contained model file (a
  ~1.1 GB llamafile, pinned and checksum-verified) into `~/.bagholder/models/` and
  runs it in the background. You install nothing and type no commands — the Summary
  cell shows a shimmer while the one-time download runs, then summaries appear.
  `localmodel.py` manages this. The Summary column itself appears only once a summary
  exists; where a document's text or the model is unavailable it stays absent rather
  than showing a wrong guess.
  - The model needs the document's text. SEC filings are HTML and summarize out of
    the box. SEDAR+ documents are PDFs whose subsetted fonts a naive reader cannot
    decode, so their text comes from `pdftext.py`: a system `pdftotext` (poppler) when
    present, otherwise `pdfminer.six`, which the app installs
    for itself in the background at startup (`deps.py`), into `~/.bagholder/pylibs/`,
    with nothing to run by hand. A summary that lands before the install finishes shows
    a shimmer, then fills. `BAGHOLDER_NO_PDF=1` turns extraction off.
  - Overrides: `BAGHOLDER_LLM_URL` (use your own local server), `BAGHOLDER_OLLAMA_MODEL`,
    `BAGHOLDER_LLAMAFILE_URL` / `BAGHOLDER_LLAMAFILE_SHA256` (a different model file).
    The download runs an executable it fetched, so it is refused unless its SHA-256
    matches the pin.

  Where the model or a document's text is unavailable the summary is simply empty
  and the subject still shows; nothing breaks.

`GET /api/filings/enrich?symbol=<symbol>&id=<item id>` reads one document and
returns `{subject, summary}` (subject is the title), cached on the row and stamped
with the enrichment logic's version so an improved version re-reads a row once. It is what the page calls when you
open a disclosure, and what Claude can call to get the gist without the full text.

## Terms of use and pacing

Everything is on demand and paced — SEDAR+ a couple of seconds between actions, SEC
under its 10-requests-per-second limit — one instrument at a time, cached so nothing
is asked twice, never a background sweep. SEDAR+'s terms permit reproducing limited
extracts for research; SEC EDGAR is open public data. If SEDAR+ ever tightens its
gate to demand a JavaScript challenge on every session, only that source is affected
and its provider would move to a licensed feed; the pipeline, the endpoint and the
other sources do not change.
