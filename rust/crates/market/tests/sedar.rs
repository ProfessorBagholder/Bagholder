//! The parsers pinned against the trimmed real
//! result rows in tests/fixtures, the navigation helpers, the scope cache, and
//! the MCP server's protocol. No network.
use bagholder_market::sedar;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

fn fixture(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures").join(name);
    std::fs::read_to_string(p).unwrap()
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

// --- ParseFilingsTest

#[test]
fn test_every_row_has_issuer_profile_file_date_and_a_document_url() {
    let rows = sedar::parse_filings(&fixture("search_documents.html"));
    assert!(!rows.is_empty(), "the fixture has result rows");
    let nine = Regex::new(r"^\d{9}$").unwrap();
    let year = Regex::new(r"\d{4}").unwrap();
    for r in &rows {
        assert!(nine.is_match(s(&r["profileNo"])), "a nine-digit profile number");
        assert!(!s(&r["issuer"]).is_empty(), "an issuer name");
        assert!(!s(&r["file"]).is_empty(), "a document file name");
        assert!(year.is_match(s(&r["submitted"])), "a submitted date carrying a year");
        assert!(s(&r["url"]).starts_with("https://www.sedarplus.ca/"), "a same-site document url");
        assert!(s(&r["url"]).contains("resource.html"), "the url is a document resource link");
    }
}

#[test]
fn test_the_first_row_is_read_exactly() {
    let rows = sedar::parse_filings(&fixture("search_documents.html"));
    let first = &rows[0];
    assert_eq!(first["profileNo"], "000026091");
    assert_eq!(first["issuer"], "Franco-Nevada Corporation (000026091)");
    assert_eq!(first["file"], "News release - English.pdf");
    assert!(s(&first["submitted"]).starts_with("13 Sep 2026"));
    assert!(s(&first["url"]).contains("drmKey="));
}

#[test]
fn test_rows_keep_the_page_order() {
    let rows = sedar::parse_filings(&fixture("search_documents.html"));
    let dates: Vec<String> = rows.iter().map(|r| s(&r["submitted"]).to_string()).collect();
    let mut sorted = dates.clone();
    sorted.sort();
    sorted.reverse();
    assert_eq!(dates, sorted, "the page lists newest first and we keep it");
}

// --- ParseReportingIssuersTest

#[test]
fn test_each_row_maps_a_name_to_a_profile_number() {
    let rows = sedar::parse_reporting_issuers(&fixture("reporting_issuers.html"));
    assert!(!rows.is_empty());
    let nine = Regex::new(r"^\d{9}$").unwrap();
    for r in &rows {
        assert!(nine.is_match(s(&r["profileNo"])));
        assert!(!s(&r["name"]).is_empty());
    }
}

#[test]
fn test_a_known_issuer_is_read_with_its_fields() {
    let rows = sedar::parse_reporting_issuers(&fixture("reporting_issuers.html"));
    let by_no: HashMap<&str, &Value> = rows.iter().map(|r| (s(&r["profileNo"]), r)).collect();
    let r = by_no.get("000010658").expect("000010658 is listed");
    assert!(s(&r["name"]).contains("01 Quantum"));
    assert!(s(&r["provinces"]).contains("ON"));
    assert_eq!(r["type"], "Company", "the type column is read, not an eligibility flag");
}

// --- EmptyAndOddInputTest

#[test]
fn test_parsers_return_empty_on_a_blank_or_errored_page() {
    assert!(sedar::parse_filings("").is_empty());
    assert!(sedar::parse_reporting_issuers("").is_empty());
    assert!(sedar::parse_filings("<div>There has been an unexpected system error.</div>").is_empty());
}

#[test]
fn test_form_fields_drops_callback_and_button_inputs() {
    let html = concat!(
        r#"<form><input name="Keep" value="v"/>"#,
        r#"<input type="submit" name="Drop"/>"#,
        r#"<input type="hidden" name="_CBNODE_" value="x"/>"#,
        r#"<select name="Pick"><option value="a">A</option><option value="b" selected>B</option></select>"#,
        r#"<input type="checkbox" name="Off"/><input type="checkbox" name="On" value="y" checked/></form>"#,
    );
    let got: HashMap<String, String> = sedar::form_fields(html).into_iter().collect();
    assert_eq!(got.get("Keep").map(|x| x.as_str()), Some("v"));
    assert_eq!(got.get("Pick").map(|x| x.as_str()), Some("b"), "the selected option is taken");
    assert_eq!(got.get("On").map(|x| x.as_str()), Some("y"), "a checked box is kept");
    assert!(!got.contains_key("Drop"));
    assert!(!got.contains_key("Off"), "an unchecked box is dropped");
    assert!(!got.contains_key("_CBNODE_"), "callback fields are set by the caller, not carried");
}

// --- NavigationHelpersTest

#[test]
fn test_search_action_is_read_from_each_service() {
    let _ = sedar::search_action(&fixture("search_documents.html"));
    let t = |a: &str, b: &str, c: &str| Some((a.to_string(), b.to_string(), c.to_string()));
    assert_eq!(
        sedar::search_action(r#"<button class="appSearchButton" onclick="x catHtmlFragmentCallback('W766','buttonPush',null,{containerNodeId:'W706'})">"#),
        t("W766", "buttonPush", "W706")
    );
    assert_eq!(
        sedar::search_action(r#"<button id="nodeW557-searchButton" onclick="x catHtmlFragmentCallback('W557','fireOnChange',null,{containerNodeId:'W553'})">"#),
        t("W557", "fireOnChange", "W553")
    );
}

#[test]
fn test_the_issuer_menu_node_is_the_issuer_not_a_header_action() {
    let html = fixture("issuer_menu.html");
    assert_eq!(sedar::issuer_menu_node(&html, Some("Shopify Inc. / Shopify Inc.")).as_deref(), Some("W1118"));
    assert_eq!(sedar::issuer_menu_node(&html, None).as_deref(), Some("W1118"));
}

#[test]
fn test_the_documents_menu_node_is_found_on_the_profile() {
    assert_eq!(sedar::docs_menu_node(&fixture("issuer_profile.html")).as_deref(), Some("W733"));
    assert_eq!(sedar::docs_menu_node("<div>no menu here</div>"), None);
}

#[test]
fn test_refresh_identity_adopts_a_new_view_instance() {
    let mut view = sedar::View {
        app: "csa-party".into(),
        inst: "oldid".into(),
        key: "oldkey".into(),
        sid: String::new(),
        page: String::new(),
        reference: String::new(),
    };
    assert!(view.refresh_identity(&fixture("issuer_profile.html")));
    assert_ne!(view.inst, "oldid", "the id is taken from the navigated page");
    assert_ne!(view.key, "oldkey");
    assert!(!view.refresh_identity("<div>a fragment with no instance</div>"));
}

// --- McpServerTest: the server binary spoken to over stdio

fn mcp(lines: &[Value]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_disclosures-mcp"))
        .env("BAGHOLDER_BROWSER", "/nonexistent/bagholder-browser")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        for l in lines {
            writeln!(stdin, "{}", l).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

const PROTOCOL_VERSION: &str = "2024-11-05";

#[test]
fn test_initialize_reports_the_protocol_and_tool_capability() {
    let r = &mcp(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})])[0];
    assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert!(r["result"]["capabilities"].get("tools").is_some());
    assert_eq!(r["result"]["serverInfo"]["name"], "disclosures");
}

#[test]
fn test_initialized_notification_gets_no_response() {
    assert!(mcp(&[json!({"jsonrpc": "2.0", "method": "notifications/initialized"})]).is_empty());
}

#[test]
fn test_tools_list_offers_the_disclosure_tools() {
    let r = &mcp(&[json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})])[0];
    let tools = r["result"]["tools"].as_array().unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| s(&t["name"])).collect();
    names.sort();
    assert_eq!(names, ["disclosures_document", "disclosures_list", "sedar_resolve_profile"]);
    for tool in tools {
        assert!(tool.get("inputSchema").is_some());
        assert!(!s(&tool["description"]).is_empty());
    }
}

#[test]
fn test_resolve_without_the_sedar_dependency_is_a_clean_tool_error() {
    // the Rust app's SEDAR+ dependency is the browser helper
    let r = &mcp(&[json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                          "params": {"name": "sedar_resolve_profile", "arguments": {"query": "Shopify"}}})])[0];
    let payload: Value = serde_json::from_str(s(&r["result"]["content"][0]["text"])).unwrap();
    assert!(s(&payload["error"]).contains("bagholder-browser"));
}

#[test]
fn test_an_unknown_method_returns_a_json_rpc_error() {
    let r = &mcp(&[json!({"jsonrpc": "2.0", "id": 4, "method": "no/such"})])[0];
    assert_eq!(r["error"]["code"], -32601);
}

// --- ScopeCacheTest

#[test]
fn test_a_second_call_uses_the_cache_and_does_not_rewalk() {
    let mut cache = sedar::ScopeCache::default();
    let mut calls: Vec<String> = vec![];
    let a = cache.get_or_walk("000012345", || { calls.push("000012345".into()); Some("<html>appDocumentLink</html>".into()) });
    let b = cache.get_or_walk("000012345", || { calls.push("000012345".into()); Some("<html>appDocumentLink</html>".into()) });
    assert_eq!(a, b);
    assert_eq!(calls, ["000012345"], "the issuer chain is walked once, then served from cache");
}

#[test]
fn test_a_failed_walk_is_not_cached() {
    let mut cache = sedar::ScopeCache::default();
    let mut calls = 0;
    assert_eq!(cache.get_or_walk("000099999", || { calls += 1; None }), None);
    assert_eq!(cache.get_or_walk("000099999", || { calls += 1; None }), None);
    assert_eq!(calls, 2, "a None result is retried, never cached");
}
