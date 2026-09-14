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


class McpServerTest(unittest.TestCase):
    def setUp(self):
        import sedar_mcp
        self.mcp = sedar_mcp

    def test_initialize_reports_the_protocol_and_tool_capability(self):
        r = self.mcp._handle({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})
        self.assertEqual(r["result"]["protocolVersion"], self.mcp.PROTOCOL_VERSION)
        self.assertIn("tools", r["result"]["capabilities"])
        self.assertEqual(r["result"]["serverInfo"]["name"], "sedar")

    def test_initialized_notification_gets_no_response(self):
        self.assertIsNone(self.mcp._handle({"jsonrpc": "2.0", "method": "notifications/initialized"}))

    def test_tools_list_offers_the_four_filing_tools(self):
        r = self.mcp._handle({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        names = {t["name"] for t in r["result"]["tools"]}
        self.assertEqual(names, {"sedar_resolve_profile", "sedar_list_filings", "sedar_newest", "sedar_download"})
        for tool in r["result"]["tools"]:
            self.assertIn("inputSchema", tool)
            self.assertTrue(tool["description"])

    def test_a_call_without_the_dependency_is_a_clean_tool_error(self):
        saved = sedar._cffi
        try:
            sedar._cffi = None
            r = self.mcp._handle({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                                  "params": {"name": "sedar_list_filings", "arguments": {"query": "Shopify"}}})
            payload = json.loads(r["result"]["content"][0]["text"])
            self.assertIn("curl_cffi", payload["error"])
        finally:
            sedar._cffi = saved

    def test_an_unknown_method_returns_a_json_rpc_error(self):
        r = self.mcp._handle({"jsonrpc": "2.0", "id": 4, "method": "no/such"})
        self.assertEqual(r["error"]["code"], -32601)


if __name__ == "__main__":
    unittest.main()
