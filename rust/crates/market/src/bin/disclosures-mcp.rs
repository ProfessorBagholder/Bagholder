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

#[derive(Debug)]
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

/// Absent or null is the empty string; anything else is its text, as the
/// person or the client typed it.
fn lenient_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match <Value as serde::Deserialize>::deserialize(d)? {
        Value::String(s) => s,
        Value::Null => String::new(),
        v => v.to_string(),
    })
}

/// As `lenient_text`, but for a required field read into `Option`: the key
/// being present at all -- null included -- is what `required` checks for,
/// never the value's own type.
fn lenient_required<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Some(lenient_text(d)?))
}

/// The name/exchange/currency hints every tool but `sedar_resolve_profile`
/// takes, alongside its own required fields.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct Meta {
    #[serde(deserialize_with = "lenient_text")]
    name: String,
    #[serde(deserialize_with = "lenient_text")]
    exchange: String,
    #[serde(deserialize_with = "lenient_text")]
    currency: String,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct ListArgs {
    #[serde(deserialize_with = "lenient_required")]
    symbol: Option<String>,
    #[serde(flatten)]
    meta: Meta,
    limit: Option<Value>,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct DocArgs {
    #[serde(deserialize_with = "lenient_required")]
    symbol: Option<String>,
    #[serde(deserialize_with = "lenient_required")]
    id: Option<String>,
    #[serde(flatten)]
    meta: Meta,
    #[serde(deserialize_with = "lenient_text")]
    dest: String,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct SedarArgs {
    #[serde(deserialize_with = "lenient_required")]
    query: Option<String>,
}

fn required(v: Option<String>, k: &str) -> Result<String, Fail> {
    v.ok_or_else(|| Fail::Other(format!("KeyError: '{}'", k)))
}

fn call(name: &str, args: &Value) -> Result<Value, Fail> {
    match name {
        "disclosures_list" => {
            let a: ListArgs = serde_json::from_value(args.clone()).unwrap_or_default();
            let symbol = required(a.symbol, "symbol")?;
            let limit = match a.limit {
                None => 100,
                Some(v) => v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).ok_or_else(|| Fail::Other(format!("ValueError: invalid literal for int(): {}", v)))?,
            };
            Ok(serde_json::to_value(disclosures::fetch(&symbol, &a.meta.name, &a.meta.exchange, &a.meta.currency, limit.max(1) as usize, "")).unwrap())
        }
        "disclosures_document" => {
            let a: DocArgs = serde_json::from_value(args.clone()).unwrap_or_default();
            let symbol = required(a.symbol, "symbol")?;
            let id = required(a.id, "id")?;
            let result = disclosures::fetch(&symbol, &a.meta.name, &a.meta.exchange, &a.meta.currency, 200, "");
            let row = result.items.into_iter().find(|i| i.id == id);
            let row = match row {
                Some(r) => r,
                None => return Ok(json!({"error": format!("no item {} for {}", disclosures::repr_quoted(&id), symbol)})),
            };
            let (data, ct) = disclosures::document(&row)?;
            let mut dest = a.dest;
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
            let a: SedarArgs = serde_json::from_value(args.clone()).unwrap_or_default();
            let query = required(a.query, "query")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tools_lists_the_three_tools_with_their_required_fields() {
        let t = tools();
        let names: Vec<&str> = t.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["disclosures_list", "disclosures_document", "sedar_resolve_profile"]);
        assert_eq!(t[0]["inputSchema"]["required"], json!(["symbol"]));
        assert_eq!(t[1]["inputSchema"]["required"], json!(["symbol", "id"]));
        assert_eq!(t[2]["inputSchema"]["required"], json!(["query"]));
        // every tool but the SEDAR lookup takes the name/exchange/currency hints
        for i in [0, 1] {
            let props = t[i]["inputSchema"]["properties"].as_object().unwrap();
            assert!(props.contains_key("name") && props.contains_key("exchange") && props.contains_key("currency"));
        }
    }

    #[test]
    fn test_a_missing_required_argument_is_a_key_error() {
        match call("disclosures_list", &json!({})) {
            Err(Fail::Other(m)) => assert_eq!(m, "KeyError: 'symbol'"),
            other => panic!("expected KeyError, got {:?}", other.map(|v| v.to_string())),
        }
        match call("disclosures_document", &json!({"symbol": "QNC"})) {
            Err(Fail::Other(m)) => assert_eq!(m, "KeyError: 'id'"),
            other => panic!("expected KeyError, got {:?}", other.map(|v| v.to_string())),
        }
    }

    #[test]
    fn test_an_unknown_tool_is_a_value_error() {
        match call("no_such_tool", &json!({})) {
            Err(Fail::Other(m)) => assert!(m.contains("unknown tool"), "{}", m),
            other => panic!("expected ValueError, got {:?}", other.map(|v| v.to_string())),
        }
    }

    #[test]
    fn test_sedar_resolve_without_the_browser_helper_answers_unavailable() {
        // this environment has no bagholder-browser helper installed
        assert!(!sedar::available());
        let out = call("sedar_resolve_profile", &json!({"query": "Acme Corp"})).unwrap();
        assert_eq!(out, json!({"error": "SEDAR+ needs the bagholder-browser helper beside this program"}));
    }

    #[test]
    fn test_initialize_and_unknown_method_envelopes() {
        let init = handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"})).unwrap();
        assert_eq!(init["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
        assert_eq!(init["id"], json!(1));
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let unknown = handle(&json!({"jsonrpc": "2.0", "id": "x", "method": "bogus"})).unwrap();
        assert_eq!(unknown["error"]["code"], json!(-32601));
        assert!(handle(&json!({"jsonrpc": "2.0", "method": "bogus"})).is_none(), "no id: a notification, never answered");
    }

    #[test]
    fn test_tools_call_wraps_the_result_as_mcp_content() {
        let msg = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "sedar_resolve_profile", "arguments": {"query": "q"}}});
        let out = handle(&msg).unwrap();
        let text = out["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("SEDAR+ needs"));
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
