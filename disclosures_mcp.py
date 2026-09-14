"""An MCP server that exposes an instrument's regulatory disclosures to Claude, over
stdio.

A thin shell around disclosures.py: it speaks the Model Context Protocol on standard
input and output (newline-delimited JSON-RPC 2.0, the stdio transport) so Claude
Desktop, Claude Code, or any MCP client can list a company's filings from every
source that covers it (SEDAR+ for Canada, SEC EDGAR for the US, more later),
merged and tagged by source and category, and download one of them. No dependency
beyond the standard library and, for SEDAR+, curl_cffi.

Register it with Claude Code:
    claude mcp add disclosures -- python3 /full/path/to/disclosures_mcp.py

Or add it to Claude Desktop's config (Settings > Developer > Edit Config),
under "mcpServers":
    "disclosures": { "command": "python3", "args": ["/full/path/to/disclosures_mcp.py"] }

Everything is on demand and paced; nothing runs on a schedule.
"""
from __future__ import annotations

import json
import os
import sys
import tempfile

import disclosures
import sedar

PROTOCOL_VERSION = "2024-11-05"
SERVER_INFO = {"name": "disclosures", "version": "1.0.0"}

_META = {
    "name": {"type": "string", "description": "The issuer's name, to seed the lookup and guard against ticker collisions."},
    "exchange": {"type": "string", "description": "The listing exchange, if known (e.g. NASDAQ, TSX)."},
    "currency": {"type": "string", "description": "The listing currency, if known (USD, CAD)."},
}

TOOLS = [
    {
        "name": "disclosures_list",
        "description": "List a company's regulatory filings from every source that covers it (SEDAR+ Canada, SEC EDGAR US), merged newest-first and tagged by source and category (Financials, Material events, Governance, Offerings, Insider & ownership, News release). Give a ticker; add name/exchange/currency when known for accuracy.",
        "inputSchema": {
            "type": "object",
            "properties": dict({
                "symbol": {"type": "string", "description": "The ticker, e.g. SHOP or NVDA."},
                "limit": {"type": "integer", "description": "Maximum items to return (default 100)."},
            }, **_META),
            "required": ["symbol"],
        },
    },
    {
        "name": "disclosures_document",
        "description": "Download one filing to a local file, given the ticker and the item's id from disclosures_list. Returns the saved path.",
        "inputSchema": {
            "type": "object",
            "properties": dict({
                "symbol": {"type": "string", "description": "The ticker the item belongs to."},
                "id": {"type": "string", "description": "The item's id from disclosures_list (e.g. 'sec:0001-…' or 'sedar:drm:…')."},
                "dest": {"type": "string", "description": "Where to save the file. Defaults to a temp file named after the id."},
            }, **_META),
            "required": ["symbol", "id"],
        },
    },
    {
        "name": "sedar_resolve_profile",
        "description": "Find the SEDAR+ reporting-issuer profile number(s) for a Canadian company by name or ticker.",
        "inputSchema": {
            "type": "object",
            "properties": {"query": {"type": "string", "description": "Issuer name or nine-digit profile number."}},
            "required": ["query"],
        },
    },
]


def _meta(args):
    return {"name": args.get("name", ""), "exchange": args.get("exchange", ""), "currency": args.get("currency", "")}


def _call(name, args):
    if name == "disclosures_list":
        return disclosures.fetch(args["symbol"], limit=int(args.get("limit", 100)), **_meta(args))
    if name == "disclosures_document":
        result = disclosures.fetch(args["symbol"], **_meta(args))
        row = next((i for i in result.get("items", []) if i.get("id") == args["id"]), None)
        if not row:
            return {"error": "no item %r for %s" % (args["id"], args["symbol"])}
        data, ct = disclosures.document(row)
        dest = args.get("dest")
        if not dest:
            base = "".join(c for c in str(args["id"]) if c.isalnum()) or "filing"
            ext = ".pdf" if "pdf" in (ct or "") else ".html" if "html" in (ct or "") else ".bin"
            dest = os.path.join(tempfile.gettempdir(), base + ext)
        with open(dest, "wb") as fh:
            fh.write(data)
        return {"path": dest, "contentType": ct, "bytes": len(data)}
    if name == "sedar_resolve_profile":
        if not sedar.available():
            return {"error": "SEDAR+ needs curl_cffi installed: pip install curl_cffi"}
        return {"profiles": sedar.resolve_profile(args["query"])}
    raise ValueError("unknown tool %r" % name)


def _result_content(payload):
    return {"content": [{"type": "text", "text": json.dumps(payload, indent=2)}]}


def _handle(msg):
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        return {"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": SERVER_INFO,
        }}
    if method in ("notifications/initialized", "initialized"):
        return None
    if method == "tools/list":
        return {"jsonrpc": "2.0", "id": mid, "result": {"tools": TOOLS}}
    if method == "tools/call":
        params = msg.get("params") or {}
        try:
            return {"jsonrpc": "2.0", "id": mid, "result": _result_content(_call(params.get("name"), params.get("arguments") or {}))}
        except disclosures.SourceUnavailable as e:
            return {"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": json.dumps({"error": str(e)})}], "isError": True}}
        except Exception as e:
            return {"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": json.dumps({"error": "%s: %s" % (type(e).__name__, e)})}], "isError": True}}
    if mid is not None:
        return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": "method not found: %s" % method}}
    return None


def main():
    out = sys.stdout
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        response = _handle(msg)
        if response is not None:
            out.write(json.dumps(response) + "\n")
            out.flush()


if __name__ == "__main__":
    main()
