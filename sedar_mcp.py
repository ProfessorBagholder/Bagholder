"""An MCP server that exposes SEDAR+ filings to Claude, over stdio.

This is a thin shell around sedar.py: it speaks the Model Context Protocol on
standard input and output (newline-delimited JSON-RPC 2.0, the stdio transport)
so Claude Desktop, Claude Code, or any MCP client can call four tools —
resolve a Canadian issuer to its SEDAR+ profile, list an issuer's filings, read
the newest filings across SEDAR+, and download one filing to a local file. No
dependency beyond the standard library and sedar.py's own optional curl_cffi.

Register it with Claude Code:
    claude mcp add sedar -- python3 /full/path/to/sedar_mcp.py

Or add it to Claude Desktop's config (Settings > Developer > Edit Config),
under "mcpServers":
    "sedar": { "command": "python3", "args": ["/full/path/to/sedar_mcp.py"] }

Everything is on demand and paced; nothing runs on a schedule.
"""
from __future__ import annotations

import json
import os
import sys
import tempfile

import sedar

PROTOCOL_VERSION = "2024-11-05"
SERVER_INFO = {"name": "sedar", "version": "1.0.0"}

TOOLS = [
    {
        "name": "sedar_resolve_profile",
        "description": "Find the SEDAR+ reporting-issuer profile number(s) for a Canadian company by name or ticker. Returns profile numbers you can pass to sedar_list_filings.",
        "inputSchema": {
            "type": "object",
            "properties": {"query": {"type": "string", "description": "Issuer name or nine-digit profile number, e.g. 'Shopify' or '000026091'."}},
            "required": ["query"],
        },
    },
    {
        "name": "sedar_list_filings",
        "description": "List a Canadian issuer's SEDAR+ filings (prospectuses, financial statements, MD&A, material change reports, news releases), newest first. Give either a profile number or a name/ticker to resolve.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Issuer name or ticker to resolve to a profile."},
                "profile_no": {"type": "string", "description": "A nine-digit SEDAR+ profile number, if known."},
                "limit": {"type": "integer", "description": "Maximum filings to return (default 100)."},
            },
        },
    },
    {
        "name": "sedar_newest",
        "description": "The newest filings across all of SEDAR+, regardless of issuer.",
        "inputSchema": {
            "type": "object",
            "properties": {"limit": {"type": "integer", "description": "How many filings to return (default 30)."}},
        },
    },
    {
        "name": "sedar_download",
        "description": "Download one SEDAR+ filing to a local file, given the document URL from a filing row. Returns the saved path.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "The document's URL from a filing row (a sedarplus.ca resource link)."},
                "dest": {"type": "string", "description": "Where to save the file. Defaults to a temp file named after the filing."},
            },
            "required": ["url"],
        },
    },
]


def _call(name, args):
    if not sedar.available():
        return {"error": "curl_cffi is not installed on the host running this server; run: pip install curl_cffi"}
    if name == "sedar_resolve_profile":
        return {"profiles": sedar.resolve_profile(args["query"])}
    if name == "sedar_list_filings":
        return sedar.list_filings(query=args.get("query"), profile_no=args.get("profile_no"),
                                  limit=int(args.get("limit", sedar.SEARCH_LIMIT)))
    if name == "sedar_newest":
        return {"filings": sedar.newest(int(args.get("limit", 30)))}
    if name == "sedar_download":
        dest = args.get("dest")
        if not dest:
            base = "".join(c for c in os.path.basename(args["url"].split("?")[0]) if c.isalnum()) or "filing"
            dest = os.path.join(tempfile.gettempdir(), base + ".pdf")
        path, ct, n = sedar.download(args["url"], dest)
        return {"path": path, "contentType": ct, "bytes": n}
    raise ValueError("unknown tool %r" % name)


def _result_content(payload):
    return {"content": [{"type": "text", "text": json.dumps(payload, indent=2)}]}


def _handle(msg):
    """Return a response dict for a request, or None for a notification."""
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
        name = params.get("name")
        args = params.get("arguments") or {}
        try:
            return {"jsonrpc": "2.0", "id": mid, "result": _result_content(_call(name, args))}
        except (sedar.ProfileNotFound, sedar.SedarUnavailable) as e:
            return {"jsonrpc": "2.0", "id": mid, "result": {"content": [{"type": "text", "text": json.dumps({"error": str(e)})}], "isError": True}}
        except Exception as e:  # a bad argument, say: report it, don't crash the server
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
