//! An MCP server that exposes an instrument's regulatory disclosures to Claude,
//! over stdio.
//!
//! A thin shell around the disclosures pipeline: it speaks the Model Context
//! Protocol on standard input and output (newline-delimited JSON-RPC 2.0, the
//! stdio transport) so Claude Desktop, Claude Code, or any MCP client can list a
//! company's filings from every source that covers it (SEDAR+ for Canada, SEC
//! EDGAR for the US), merged and tagged by source and category, and download one.
//!
//! Register it with Claude Code:
//!     claude mcp add disclosures -- /full/path/to/disclosures-mcp
//!
//! Everything is on demand and paced; nothing runs on a schedule.

use bagholder_market::disclosures::{self, SourceError};
use bagholder_market::sedar;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

fn meta_props(mut props: serde_json::Map<String, Value>) -> Value {
    props.insert("name".into(), json!({"type": "string", "description": "The issuer's name, to seed the lookup and guard against ticker collisions."}));
    props.insert("exchange".into(), json!({"type": "string", "description": "The listing exchange, if known (e.g. NASDAQ, TSX)."}));
    props.insert("currency".into(), json!({"type": "string", "description": "The listing currency, if known (USD, CAD)."}));
    Value::Object(props)
}

fn tools() -> Value {
    let list_props = json!({
        "symbol": {"type": "string", "description": "The ticker, e.g. SHOP or NVDA."},
        "limit": {"type": "integer", "description": "Maximum items to return (default 100)."},
    });
    let doc_props = json!({
        "symbol": {"type": "string", "description": "The ticker the item belongs to."},
        "id": {"type": "string", "description": "The item's id from disclosures_list (e.g. 'sec:0001-…' or 'sedar:drm:…')."},
        "dest": {"type": "string", "description": "Where to save the file. Defaults to a temp file named after the id."},
    });
    json!([
        {
            "name": "disclosures_list",
            "description": "List a company's regulatory filings from every source that covers it (SEDAR+ Canada, SEC EDGAR US), merged newest-first and tagged by source and category (Financials, Material events, Governance, Offerings, Insider & ownership, News release). Give a ticker; add name/exchange/currency when known for accuracy.",
            "inputSchema": {"type": "object", "properties": meta_props(list_props.as_object().unwrap().clone()), "required": ["symbol"]},
        },
        {
            "name": "disclosures_document",
            "description": "Download one filing to a local file, given the ticker and the item's id from disclosures_list. Returns the saved path.",
            "inputSchema": {"type": "object", "properties": meta_props(doc_props.as_object().unwrap().clone()), "required": ["symbol", "id"]},
        },
        {
            "name": "sedar_resolve_profile",
            "description": "Find the SEDAR+ reporting-issuer profile number(s) for a Canadian company by name or ticker.",
            "inputSchema": {"type": "object", "properties": {"query": {"type": "string", "description": "Issuer name or nine-digit profile number."}}, "required": ["query"]},
        },
    ])
}

enum Fail {
    Source(String),
    Other(String),
}

impl From<SourceError> for Fail {
    fn from(e: SourceError) -> Fail {
        match e {
            SourceError::Unavailable(m) => Fail::Source(m),
            SourceError::Other(m) => Fail::Other(m),
        }
    }
}

fn arg(args: &Value, k: &str) -> String {
    match args.get(k) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn required(args: &Value, k: &str) -> Result<String, Fail> {
    if args.get(k).is_none() {
        return Err(Fail::Other(format!("KeyError: '{}'", k)));
    }
    Ok(arg(args, k))
}

fn call(name: &str, args: &Value) -> Result<Value, Fail> {
    let (nm, ex, cur) = (arg(args, "name"), arg(args, "exchange"), arg(args, "currency"));
    match name {
        "disclosures_list" => {
            let symbol = required(args, "symbol")?;
            let limit = match args.get("limit") {
                None => 100,
                Some(v) => v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).ok_or_else(|| Fail::Other(format!("ValueError: invalid literal for int(): {}", v)))?,
            };
            Ok(disclosures::fetch(&symbol, &nm, &ex, &cur, limit.max(1) as usize, ""))
        }
        "disclosures_document" => {
            let symbol = required(args, "symbol")?;
            let id = required(args, "id")?;
            let result = disclosures::fetch(&symbol, &nm, &ex, &cur, 200, "");
            let row = result.get("items").and_then(|i| i.as_array()).and_then(|items| items.iter().find(|i| i.get("id").and_then(|v| v.as_str()) == Some(id.as_str())).cloned());
            let row = match row {
                Some(r) => r,
                None => return Ok(json!({"error": format!("no item {} for {}", disclosures::repr_quoted(&id), symbol)})),
            };
            let (data, ct) = disclosures::document(&row)?;
            let mut dest = arg(args, "dest");
            if dest.is_empty() {
                let base: String = id.chars().filter(|c| c.is_alphanumeric()).collect();
                let base = if base.is_empty() { "filing".to_string() } else { base };
                let ext = if ct.contains("pdf") { ".pdf" } else if ct.contains("html") { ".html" } else { ".bin" };
                dest = std::env::temp_dir().join(base + ext).to_string_lossy().into_owned();
            }
            std::fs::write(&dest, &data).map_err(|e| Fail::Other(format!("OSError: {}", e)))?;
            Ok(json!({"path": dest, "contentType": ct, "bytes": data.len()}))
        }
        "sedar_resolve_profile" => {
            let query = required(args, "query")?;
            if !sedar::available() {
                return Ok(json!({"error": "SEDAR+ needs the bagholder-browser helper beside this program"}));
            }
            match sedar::resolve_profile(&query)? {
                sedar::Lookup::Found(rows) => Ok(json!({"profiles": rows})),
                sedar::Lookup::NotFound => {
                    let q = query.trim();
                    Err(Fail::Other(if q.is_empty() { "ProfileNotFound: empty query".into() } else { format!("ProfileNotFound: no SEDAR+ profile matched {}", disclosures::repr_quoted(q)) }))
                }
            }
        }
        _ => Err(Fail::Other(format!("ValueError: unknown tool {}", disclosures::repr_quoted(name)))),
    }
}

fn handle(msg: &Value) -> Option<Value> {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let mid = msg.get("id").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => Some(json!({"jsonrpc": "2.0", "id": mid, "result": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "disclosures", "version": "1.0.0"},
        }})),
        "notifications/initialized" | "initialized" => None,
        "tools/list" => Some(json!({"jsonrpc": "2.0", "id": mid, "result": {"tools": tools()}})),
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(json!({}));
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let args = params.get("arguments").filter(|a| a.is_object()).cloned().unwrap_or(json!({}));
            let result = match call(&name, &args) {
                Ok(payload) => json!({"content": [{"type": "text", "text": serde_json::to_string_pretty(&payload).unwrap_or_default()}]}),
                Err(Fail::Source(m)) | Err(Fail::Other(m)) => json!({"content": [{"type": "text", "text": json!({"error": m}).to_string()}], "isError": true}),
            };
            Some(json!({"jsonrpc": "2.0", "id": mid, "result": result}))
        }
        _ if !mid.is_null() => Some(json!({"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": format!("method not found: {}", method)}})),
        _ => None,
    }
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(line) { Ok(m) => m, Err(_) => continue };
        if let Some(resp) = handle(&msg) {
            let _ = writeln!(out, "{}", resp);
            let _ = out.flush();
        }
    }
}
