"""EDGAR provider and the disclosures dispatcher. No network: the SEC ticker map
and submissions payload are stubbed, so only the pure logic (categories, name
guard, normalization, merge) is exercised."""
from __future__ import annotations

import unittest

import disclosures
import edgar


SUBMISSIONS = {
    "name": "NVIDIA CORP",
    "filings": {"recent": {
        "form": ["10-Q", "8-K", "4", "SCHEDULE 13D/A", "424B5", "DEF 14A", "NT 10-K"],
        "filingDate": ["2026-08-05", "2026-08-01", "2026-07-30", "2026-07-20", "2026-07-10", "2026-06-15", "2026-06-01"],
        "primaryDocument": ["nvda-10q.htm", "nvda-8k.htm", "form4.xml", "sc13da.htm", "424b5.htm", "proxy.htm", ""],
        "accessionNumber": ["0001-26-01", "0001-26-02", "0001-26-03", "0001-26-04", "0001-26-05", "0001-26-06", "0001-26-07"],
        "primaryDocDescription": ["", "", "", "", "", "", ""],
    }},
}


class EdgarUnitTest(unittest.TestCase):
    def setUp(self):
        self._tickers = edgar._tickers
        self._get_json = edgar._get_json
        edgar._tickers = {"NVDA": (1045810, "NVIDIA CORP"), "SHOP": (1594805, "SHOPIFY INC.")}
        edgar._get_json = lambda url: SUBMISSIONS

    def tearDown(self):
        edgar._tickers = self._tickers
        edgar._get_json = self._get_json

    def test_bare_ticker_strips_venue_and_dots(self):
        self.assertEqual(edgar._bare("SHOP.TO"), "SHOP")
        self.assertEqual(edgar._bare("BRK.B"), "BRK-B")
        self.assertEqual(edgar._bare("nvda"), "NVDA")

    def test_forms_map_to_the_shared_categories(self):
        self.assertEqual(edgar._category("10-Q"), disclosures.FINANCIALS)
        self.assertEqual(edgar._category("40-F"), disclosures.FINANCIALS)
        self.assertEqual(edgar._category("8-K"), disclosures.EVENTS)
        self.assertEqual(edgar._category("DEF 14A"), disclosures.GOVERNANCE)
        self.assertEqual(edgar._category("424B5"), disclosures.OFFERINGS)
        self.assertEqual(edgar._category("4"), disclosures.INSIDER)
        self.assertEqual(edgar._category("SCHEDULE 13D/A"), disclosures.INSIDER)
        self.assertEqual(edgar._category("NT 10-K"), disclosures.OTHER)

    def test_covers_a_us_listing_and_a_known_ticker(self):
        self.assertTrue(edgar.covers("NVDA", "NASDAQ", "USD"))
        self.assertTrue(edgar.covers("SHOP", "TSX", "CAD"), "a cross-listed ticker SEC knows")
        self.assertFalse(edgar.covers("QNC", "TSX-V", "CAD"), "a pure-Canadian ticker SEC does not know")

    def test_fetch_normalizes_rows_with_source_and_url(self):
        items = edgar.fetch("NVDA", name="NVIDIA Corporation", exchange="NASDAQ", currency="USD")
        self.assertEqual(len(items), 7)
        first = items[0]
        self.assertEqual(first["source"], "SEC")
        self.assertEqual(first["id"], "sec:0001-26-01")
        self.assertEqual(first["type"], "10-Q")
        self.assertEqual(first["category"], disclosures.FINANCIALS)
        self.assertTrue(first["url"].startswith("https://www.sec.gov/Archives/edgar/data/1045810/000126"))

    def test_a_missing_document_url_falls_back_to_the_company_page(self):
        items = edgar.fetch("NVDA", exchange="NASDAQ", currency="USD")
        nt = next(i for i in items if i["type"] == "NT 10-K")
        self.assertIn("browse-edgar", nt["url"])

    def test_name_guard_rejects_a_canadian_ticker_colliding_with_a_us_filer(self):
        # A Canadian instrument named nothing like the US SEC entity is dropped.
        items = edgar.fetch("NVDA", name="Northvolt Canada Mining Corp.", exchange="TSX-V", currency="CAD")
        self.assertEqual(items, [], "the SEC 'NVIDIA CORP' entity does not match the Canadian name")

    def test_unknown_ticker_returns_empty(self):
        self.assertEqual(edgar.fetch("ZZZZ", exchange="NASDAQ", currency="USD"), [])


class NameMatchTest(unittest.TestCase):
    def test_matches_ignore_corporate_suffixes_and_case(self):
        self.assertTrue(disclosures.names_match("Shopify Inc.", "SHOPIFY INC."))
        self.assertTrue(disclosures.names_match("NVIDIA Corporation", "NVIDIA CORP"))

    def test_unrelated_names_do_not_match(self):
        self.assertFalse(disclosures.names_match("Quantum eMotion Corp.", "QUALCOMM INC"))
        self.assertFalse(disclosures.names_match("", "Anything"))


class DispatcherTest(unittest.TestCase):
    """Merge and per-source status, with fake providers so the logic is isolated."""

    def setUp(self):
        self._providers = disclosures.PROVIDERS

        class Prov:
            def __init__(self, source, items, avail=True, covers=True, raises=None):
                self.SOURCE = source
                self._items = items
                self._avail = avail
                self._covers = covers
                self._raises = raises

            def available(self):
                return self._avail

            def covers(self, symbol, exchange="", currency=""):
                return self._covers

            def fetch(self, symbol, name="", exchange="", currency="", limit=200):
                if self._raises:
                    raise self._raises
                return list(self._items)

            def document(self, row):
                return b"%PDF-", "application/pdf"

        self.Prov = Prov

    def tearDown(self):
        disclosures.PROVIDERS = self._providers

    def test_items_from_both_providers_merge_newest_first(self):
        a = self.Prov("A", [{"id": "a:1", "source": "A", "date": "2026-01-01"}])
        b = self.Prov("B", [{"id": "b:1", "source": "B", "date": "2026-05-01"}])
        disclosures.PROVIDERS = [a, b]
        out = disclosures.fetch("X")
        self.assertEqual([i["id"] for i in out["items"]], ["b:1", "a:1"])
        self.assertTrue(out["sources"]["A"]["matched"])
        self.assertTrue(out["sources"]["B"]["matched"])

    def test_a_failing_source_is_recorded_and_the_other_still_returns(self):
        a = self.Prov("A", [], raises=disclosures.SourceUnavailable("down"))
        b = self.Prov("B", [{"id": "b:1", "source": "B", "date": "2026-05-01"}])
        disclosures.PROVIDERS = [a, b]
        out = disclosures.fetch("X")
        self.assertEqual([i["id"] for i in out["items"]], ["b:1"])
        self.assertFalse(out["sources"]["A"]["available"])
        self.assertIn("down", out["sources"]["A"]["error"])

    def test_a_source_that_does_not_cover_is_skipped(self):
        a = self.Prov("A", [{"id": "a:1", "source": "A", "date": "2026-01-01"}], covers=False)
        disclosures.PROVIDERS = [a]
        out = disclosures.fetch("X")
        self.assertEqual(out["items"], [])
        self.assertFalse(out["sources"]["A"]["matched"])

    def test_document_routes_to_the_rows_source(self):
        a = self.Prov("A", [])
        disclosures.PROVIDERS = [a]
        data, ct = disclosures.document({"source": "A", "id": "a:1"})
        self.assertEqual(ct, "application/pdf")




class ContentResolutionTest(unittest.TestCase):
    """content() reads a filing's substance: the largest real document in the accession,
    skipping the cover form, the index files, and the full-submission dump."""
    def setUp(self):
        self._json, self._doc = edgar._get_json, edgar.document
    def tearDown(self):
        edgar._get_json, edgar.document = self._json, self._doc

    def test_it_picks_the_largest_substantive_document(self):
        base = "https://www.sec.gov/Archives/edgar/data/2106613/000110465926097327"
        edgar._get_json = lambda url: {"directory": {"item": [
            {"name": "0001104659-26-097327-index.html", "size": 3000},
            {"name": "0001104659-26-097327.txt", "size": 106958},   # full submission dump, skipped
            {"name": "tm2623033d1_6k.htm", "size": 1230},           # cover form
            {"name": "tm2623033d1_ex99-1.htm", "size": 62339},      # the MD&A -> chosen
            {"name": "tm2623033d1_ex99-3.htm", "size": 1223},
        ]}}
        seen = {}
        edgar.document = lambda row: (seen.setdefault("url", row["url"]), (b"<html>MD&A</html>", "text/html"))[1]
        data, ct = edgar.content({"url": base + "/tm2623033d1_6k.htm", "source": "SEC"})
        self.assertTrue(seen["url"].endswith("tm2623033d1_ex99-1.htm"))

    def test_it_falls_back_to_the_primary_when_the_index_is_unavailable(self):
        edgar._get_json = lambda url: (_ for _ in ()).throw(OSError("no index"))
        called = {}
        edgar.document = lambda row: (called.setdefault("url", row["url"]), (b"x", "text/html"))[1]
        edgar.content({"url": "https://www.sec.gov/Archives/edgar/data/1/2/primary.htm", "source": "SEC"})
        self.assertTrue(called["url"].endswith("primary.htm"))




class OwnershipEnrichmentTest(unittest.TestCase):
    """A Schedule 13G is parsed to an exact title/summary from its XML, not the model."""
    def setUp(self):
        self._doc = edgar.document
    def tearDown(self):
        edgar.document = self._doc

    def test_13g_yields_a_stake_title_and_summary(self):
        xml = (b"<edgarSubmission><submissionType>SCHEDULE 13G/A</submissionType>"
               b"<issuerName>Quantum eMotion Corp.</issuerName>"
               b"<reportingPersonName>Capital Ventures International</reportingPersonName>"
               b"<reportingPersonName>Susquehanna Advisors Group, Inc.</reportingPersonName>"
               b"<classPercent>2.4</classPercent></edgarSubmission>")
        seen = {}
        edgar.document = lambda row: (seen.setdefault("url", row["url"]), (xml, "application/xml"))[1]
        out = edgar.enrichment({"type": "SCHEDULE 13G/A", "source": "SEC",
                                "url": "https://www.sec.gov/Archives/edgar/data/1/2/xslSCHEDULE_13G_X02/primary_doc.xml"})
        self.assertNotIn("xsl", seen["url"])   # the raw XML is read, not the rendered page
        self.assertIn("2.4%", out["subject"])
        self.assertIn("Capital Ventures International", out["subject"])
        self.assertIn("2.4% of Quantum eMotion Corp.", out["summary"])

    def test_non_ownership_forms_defer_to_the_model(self):
        self.assertIsNone(edgar.enrichment({"type": "6-K", "source": "SEC", "url": "https://www.sec.gov/x/y.htm"}))


if __name__ == "__main__":
    unittest.main()
