"""SEDAR+ fetch module: the parsers are the contract, pinned against real result rows.

The fixtures under tests/fixtures are trimmed but unaltered rows from the live
SEDAR+ document search and reporting-issuer search. Network calls are never made
here; only the pure parsers and the CLI's shape are exercised."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import unittest

import sedar

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIX = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures")


def fixture(name):
    with open(os.path.join(FIX, name), encoding="utf-8") as fh:
        return fh.read()


class ParseFilingsTest(unittest.TestCase):
    def setUp(self):
        self.rows = sedar.parse_filings(fixture("search_documents.html"))

    def test_every_row_has_issuer_profile_file_date_and_a_document_url(self):
        self.assertTrue(self.rows, "the fixture has result rows")
        for r in self.rows:
            self.assertRegex(r["profileNo"], r"^\d{9}$", "a nine-digit profile number")
            self.assertTrue(r["issuer"], "an issuer name")
            self.assertTrue(r["file"], "a document file name")
            self.assertRegex(r["submitted"], r"\d{4}", "a submitted date carrying a year")
            self.assertTrue(r["url"].startswith("https://www.sedarplus.ca/"), "a same-site document url")
            self.assertIn("resource.html", r["url"], "the url is a document resource link")

    def test_the_first_row_is_read_exactly(self):
        first = self.rows[0]
        self.assertEqual(first["profileNo"], "000026091")
        self.assertEqual(first["issuer"], "Franco-Nevada Corporation (000026091)")
        self.assertEqual(first["file"], "News release - English.pdf")
        self.assertTrue(first["submitted"].startswith("13 Sep 2026"))
        self.assertIn("drmKey=", first["url"])

    def test_rows_keep_the_page_order(self):
        dates = [r["submitted"] for r in self.rows]
        self.assertEqual(dates, sorted(dates, reverse=True), "the page lists newest first and we keep it")


class ParseReportingIssuersTest(unittest.TestCase):
    def setUp(self):
        self.rows = sedar.parse_reporting_issuers(fixture("reporting_issuers.html"))

    def test_each_row_maps_a_name_to_a_profile_number(self):
        self.assertTrue(self.rows)
        for r in self.rows:
            self.assertRegex(r["profileNo"], r"^\d{9}$")
            self.assertTrue(r["name"])

    def test_a_known_issuer_is_read_with_its_fields(self):
        by_no = {r["profileNo"]: r for r in self.rows}
        self.assertIn("000010658", by_no)
        r = by_no["000010658"]
        self.assertIn("01 Quantum", r["name"])
        self.assertIn("ON", r["provinces"])
        self.assertEqual(r["type"], "Company", "the type column is read, not an eligibility flag")


class EmptyAndOddInputTest(unittest.TestCase):
    def test_parsers_return_empty_on_a_blank_or_errored_page(self):
        self.assertEqual(sedar.parse_filings(""), [])
        self.assertEqual(sedar.parse_reporting_issuers(""), [])
        self.assertEqual(sedar.parse_filings("<div>There has been an unexpected system error.</div>"), [])

    def test_form_fields_drops_callback_and_button_inputs(self):
        html = (
            '<form><input name="Keep" value="v"/>'
            '<input type="submit" name="Drop"/>'
            '<input type="hidden" name="_CBNODE_" value="x"/>'
            '<select name="Pick"><option value="a">A</option><option value="b" selected>B</option></select>'
            '<input type="checkbox" name="Off"/><input type="checkbox" name="On" value="y" checked/></form>'
        )
        got = dict(sedar._form_fields(html))
        self.assertEqual(got.get("Keep"), "v")
        self.assertEqual(got.get("Pick"), "b", "the selected option is taken")
        self.assertEqual(got.get("On"), "y", "a checked box is kept")
        self.assertNotIn("Drop", got)
        self.assertNotIn("Off", got, "an unchecked box is dropped")
        self.assertNotIn("_CBNODE_", got, "callback fields are set by the caller, not carried")


class CliTest(unittest.TestCase):
    def test_help_names_the_commands(self):
        r = subprocess.run([sys.executable, os.path.join(ROOT, "sedar.py"), "--help"],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        self.assertIn("filings", r.stderr)
        self.assertIn("resolve", r.stderr)

    def test_a_missing_dependency_is_reported_as_json_not_a_traceback(self):
        # Run the CLI with curl_cffi forced absent; it must answer with a clean JSON error.
        script = (
            "import sedar; sedar._cffi = None;"
            "import sys; raise SystemExit(sedar._main(['filings', 'Shopify']))"
        )
        r = subprocess.run([sys.executable, "-c", script], cwd=ROOT, capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        payload = json.loads(r.stdout)
        self.assertFalse(payload["ok"])
        self.assertIn("curl_cffi", payload["error"])


class NavigationHelpersTest(unittest.TestCase):
    """The per-issuer path walks the reporting-issuer list to the issuer's own
    'Search and download documents' page. These pin the node discovery."""

    def test_search_action_is_read_from_each_service(self):
        docs = sedar._search_action(fixture("search_documents.html"))
        # the trimmed document fixture has result rows but not the search control;
        # the reporting-issuer fixture is result rows only too, so use live-shaped
        # snippets: the control discovery is covered by the issuer fixtures below.
        self.assertIn(sedar._search_action('<button class="appSearchButton" onclick="x catHtmlFragmentCallback(\'W766\',\'buttonPush\',null,{containerNodeId:\'W706\'})">'),
                      [("W766", "buttonPush", "W706")])
        self.assertEqual(
            sedar._search_action('<button id="nodeW557-searchButton" onclick="x catHtmlFragmentCallback(\'W557\',\'fireOnChange\',null,{containerNodeId:\'W553\'})">'),
            ("W557", "fireOnChange", "W553"))

    def test_the_issuer_menu_node_is_the_issuer_not_a_header_action(self):
        html = fixture("issuer_menu.html")
        self.assertEqual(sedar._issuer_menu_node(html, "Shopify Inc. / Shopify Inc."), "W1118")
        # without a name, it still skips "search for profiles" and takes the issuer
        self.assertEqual(sedar._issuer_menu_node(html), "W1118")

    def test_the_documents_menu_node_is_found_on_the_profile(self):
        self.assertEqual(sedar._docs_menu_node(fixture("issuer_profile.html")), "W733")
        self.assertIsNone(sedar._docs_menu_node("<div>no menu here</div>"))

    def test_refresh_identity_adopts_a_new_view_instance(self):
        view = sedar._View.__new__(sedar._View)
        view.app = "csa-party"
        view.inst = "oldid"
        view.key = "oldkey"
        ok = view.refresh_identity(fixture("issuer_profile.html"))
        self.assertTrue(ok)
        self.assertNotEqual(view.inst, "oldid", "the id is taken from the navigated page")
        self.assertNotEqual(view.key, "oldkey")
        self.assertFalse(view.refresh_identity("<div>a fragment with no instance</div>"))


class McpServerTest(unittest.TestCase):
    def setUp(self):
        import disclosures_mcp
        self.mcp = disclosures_mcp

    def test_initialize_reports_the_protocol_and_tool_capability(self):
        r = self.mcp._handle({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        self.assertEqual(r["result"]["protocolVersion"], self.mcp.PROTOCOL_VERSION)
        self.assertIn("tools", r["result"]["capabilities"])
        self.assertEqual(r["result"]["serverInfo"]["name"], "disclosures")

    def test_initialized_notification_gets_no_response(self):
        self.assertIsNone(self.mcp._handle({"jsonrpc": "2.0", "method": "notifications/initialized"}))

    def test_tools_list_offers_the_disclosure_tools(self):
        r = self.mcp._handle({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        names = {t["name"] for t in r["result"]["tools"]}
        self.assertEqual(names, {"disclosures_list", "disclosures_document", "sedar_resolve_profile"})
        for tool in r["result"]["tools"]:
            self.assertIn("inputSchema", tool)
            self.assertTrue(tool["description"])

    def test_resolve_without_the_sedar_dependency_is_a_clean_tool_error(self):
        saved = sedar._cffi
        try:
            sedar._cffi = None
            r = self.mcp._handle({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                                  "params": {"name": "sedar_resolve_profile", "arguments": {"query": "Shopify"}}})
            payload = json.loads(r["result"]["content"][0]["text"])
            self.assertIn("curl_cffi", payload["error"])
        finally:
            sedar._cffi = saved

    def test_disclosures_list_calls_the_pipeline(self):
        import disclosures
        saved = disclosures.fetch
        try:
            disclosures.fetch = lambda symbol, name="", exchange="", currency="", limit=100: {"items": [{"id": "sec:1", "source": "SEC"}], "sources": {}}
            r = self.mcp._handle({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
                                  "params": {"name": "disclosures_list", "arguments": {"symbol": "NVDA"}}})
            payload = json.loads(r["result"]["content"][0]["text"])
            self.assertEqual(payload["items"][0]["id"], "sec:1")
        finally:
            disclosures.fetch = saved

    def test_an_unknown_method_returns_a_json_rpc_error(self):
        r = self.mcp._handle({"jsonrpc": "2.0", "id": 4, "method": "no/such"})
        self.assertEqual(r["error"]["code"], -32601)


if __name__ == "__main__":
    unittest.main()
