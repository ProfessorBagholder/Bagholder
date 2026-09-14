"""The filings store table and the app's filings glue: caching, staleness, and the
payload the endpoint returns. The source itself (sedar.py) is stubbed so no network
is touched; its parsers are covered in test_sedar."""
from __future__ import annotations

import os
import tempfile
import unittest
from datetime import datetime, timedelta, timezone

import bagholder
import sedar
import store


def sample(profile="000026091", n=3):
    rows = []
    for i in range(n):
        url = "https://www.sedarplus.ca/csa-party/viewInstance/resource.html?node=W8%02d&drmKey=abc%03d&id=xy" % (i, i)
        rows.append({
            "id": sedar.filing_id(url),
            "profileNo": profile,
            "issuer": "Example Corp. (%s)" % profile,
            "file": "Document %d.pdf" % i,
            "submitted": "1%d Sep 2026 10:00 EDT" % i,
            "submittedAt": "2026-09-1%dT10:00" % i,
            "size": "100 KB",
            "url": url,
        })
    return rows


class FilingsStoreTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_rows_are_kept_per_symbol_newest_first_and_the_profile_is_remembered(self):
        store.replace_filings("SHOP", "000026091", sample(n=3))
        got = store.filings("SHOP")
        self.assertEqual(len(got), 3)
        self.assertEqual([f["submittedAt"] for f in got], sorted([f["submittedAt"] for f in got], reverse=True))
        self.assertEqual(store.sedar_profile("SHOP"), "000026091")
        self.assertTrue(store.filings_fetched_at("SHOP"))

    def test_a_second_fetch_replaces_rather_than_doubles(self):
        store.replace_filings("SHOP", "000026091", sample(n=3))
        store.replace_filings("SHOP", "000026091", sample(n=2))
        self.assertEqual(len(store.filings("SHOP")), 2, "the same symbol's rows are replaced, not appended")

    def test_symbols_do_not_bleed_into_each_other(self):
        store.replace_filings("SHOP", "000026091", sample(profile="000026091", n=2))
        store.replace_filings("ATD", "000012345", sample(profile="000012345", n=3))
        self.assertEqual(len(store.filings("SHOP")), 2)
        self.assertEqual(len(store.filings("ATD")), 3)
        self.assertEqual(store.sedar_profile("ATD"), "000012345")

    def test_forget_clears_rows_and_the_stamp(self):
        store.replace_filings("SHOP", "000026091", sample(n=2))
        store.forget_filings("SHOP")
        self.assertEqual(store.filings("SHOP"), [])
        self.assertEqual(store.filings_fetched_at("SHOP"), "")
        self.assertEqual(store.sedar_profile("SHOP"), "")

    def test_data_summary_counts_filings(self):
        store.replace_filings("SHOP", "000026091", sample(n=3))
        self.assertEqual(store.data_summary()["filings"], 3)


class FilingsPayloadTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        self._avail = sedar.available
        self._list = sedar.list_filings

    def tearDown(self):
        sedar.available = self._avail
        sedar.list_filings = self._list
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_stale_is_true_until_a_fetch_then_false_within_a_day(self):
        self.assertTrue(bagholder._filings_stale("SHOP"))
        store.replace_filings("SHOP", "000026091", sample(n=1))
        self.assertFalse(bagholder._filings_stale("SHOP"))

    def test_a_day_old_stamp_is_stale(self):
        old = (datetime.now(timezone.utc) - timedelta(hours=25)).strftime("%Y-%m-%dT%H:%M:%SZ")
        store.replace_filings("SHOP", "000026091", sample(n=1), now=old)
        self.assertTrue(bagholder._filings_stale("SHOP"))

    def test_payload_refreshes_when_forced_and_reads_the_source(self):
        sedar.available = lambda: True
        sedar.list_filings = lambda query=None, profile_no=None, limit=100: {
            "profile": {"profileNo": "000026091", "name": "Shopify Inc."},
            "filings": sample(n=4),
        }
        out = bagholder.filings_payload("SHOP", refresh=True)
        self.assertTrue(out["ok"])
        self.assertTrue(out["available"])
        self.assertTrue(out["refreshed"])
        self.assertEqual(out["profileNo"], "000026091")
        self.assertEqual(len(out["filings"]), 4)

    def test_payload_reports_when_the_source_is_unavailable(self):
        sedar.available = lambda: False
        out = bagholder.filings_payload("NVDA", refresh=True)
        self.assertTrue(out["ok"], "the endpoint still answers cleanly")
        self.assertFalse(out["available"])
        self.assertTrue(out["sourceUnavailable"])
        self.assertEqual(out["filings"], [])

    def test_a_missing_profile_leaves_an_empty_but_stamped_result(self):
        sedar.available = lambda: True
        def raise_notfound(query=None, profile_no=None, limit=100):
            raise sedar.ProfileNotFound("no match")
        sedar.list_filings = raise_notfound
        out = bagholder.filings_payload("ZZZZ", refresh=True)
        self.assertTrue(out["ok"])
        self.assertEqual(out["filings"], [])
        self.assertTrue(store.filings_fetched_at("ZZZZ"), "the attempt is stamped so it is not retried every open")

    def test_empty_symbol_is_rejected(self):
        self.assertFalse(bagholder.filings_payload("")["ok"])


if __name__ == "__main__":
    unittest.main()
