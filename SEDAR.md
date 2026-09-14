# SEDAR+ filings

SEDAR+ (`sedarplus.ca`) is the Canadian securities filing system: every reporting
issuer's prospectuses, financial statements, MD&A, material change reports and
news releases are filed there. It has no public API and sits behind Radware Bot
Manager, which turns away ordinary HTTP clients and every headless browser at the
TLS handshake. A browser's own TLS fingerprint is what the gate checks, so
`sedar.py` reaches the site through `curl_cffi` with `impersonate="chrome"`, holds
one paced session, resolves an issuer to its nine-digit profile number, lists that
profile's filings, and downloads a filing as its PDF. Every request is on demand
and paced a couple of seconds apart; nothing sweeps or runs on a schedule.

`curl_cffi` is the one dependency. It installs with `pip install -r requirements.txt`
and ships in the Docker image. Where it is absent the feature reports itself
unavailable and the rest of Bagholder is unchanged.

## Three ways to reach it

### 1. The running app's endpoint

While `bagholder.py` is running (default `http://127.0.0.1:8765`):

    GET /api/filings?symbol=SHOP            an instrument's filings, from the cache
    GET /api/filings?symbol=SHOP&refresh=1  fetch first, then return them
    GET /api/filings?symbol=SHOP&name=Shopify%20Inc.   seed the issuer lookup with a name
    GET /api/filings/doc?symbol=SHOP&id=<filing id>  the filing itself, as application/pdf

The list is cached per symbol and refreshed when it is asked for and the stored
copy is more than a day old, or when `refresh=1` is passed. A filing never changes
once filed, so a downloaded document is final.

### 2. The command line (for Claude Code, or a shell)

    python3 sedar.py resolve "Shopify"          profiles matching an issuer
    python3 sedar.py filings "Shopify" 50        an issuer's 50 newest filings (JSON)
    python3 sedar.py newest 30                   the newest filings across SEDAR+
    python3 sedar.py get <profileNo> <filing id> out.pdf   download one document

Each command prints JSON on stdout, so Claude Code can call it and read the result
directly.

### 3. The MCP server (for Claude Desktop, Claude Code, or any MCP client)

`sedar_mcp.py` exposes four tools — `sedar_resolve_profile`, `sedar_list_filings`,
`sedar_newest`, `sedar_download` — over stdio.

Claude Code:

    claude mcp add sedar -- python3 /full/path/to/sedar_mcp.py

Claude Desktop (Settings > Developer > Edit Config), under `mcpServers`:

    "sedar": { "command": "python3", "args": ["/full/path/to/sedar_mcp.py"] }

To install it as a one-click Claude Desktop extension, pack `mcp/manifest.json`
with the `mcpb` CLI (`npx @anthropic-ai/mcpb pack`) and open the resulting
`.mcpb` file; the manifest is already written for `sedar_mcp.py`.

## Terms of use

SEDAR+'s terms permit reproducing limited unaltered extracts of the public
information for research or internal use, and draw the line at automated searches
that would burden the site. This tool acts only on a request you make, at a
person's pace, and caches so nothing is asked twice; it never monitors or sweeps.
Whether to use it is your call. If the site ever tightens its gate to demand a
JavaScript challenge on every session, no HTTP client will pass, and the source
inside `sedar.py` would have to move to a licensed feed (for example QuoteMedia).
