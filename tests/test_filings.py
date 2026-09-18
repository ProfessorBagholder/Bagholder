"""The disclosures store and the app's filings glue: per-source caching, merge,
staleness, and the payload the endpoint returns. The providers are stubbed so no
network is touched; their parsing is covered in test_sedar and test_edgar."""
from __future__ import annotations

import os
import tempfile
import unittest
from datetime import datetime, timedelta, timezone
from unittest import mock

import bagholder
import disclosures
import store


def item(source="SEC", category="Financials", i=0, profile=""):
    tag = source.split("+")[0].lower().replace(" ", "")
    return {
        "id": "%s:%d" % (tag, i),
        "source": source,
        "category": category,
        "date": "2026-08-%02d" % (10 + i),
        "dateText": "2026-08-%02d" % (10 + i),
        "type": "10-Q" if source == "SEC" else "Interim MD&A",
        "title": "Quarterly report" if source == "SEC" else "",
        "size": "" if source == "SEC" else "292 KB",
        "url": "https://www.sec.gov/x/%d" % i if source == "SEC" else "https://www.sedarplus.ca/x?drmKey=%d" % i,
        "profileNo": profile,
    }


class DisclosuresStoreTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_rows_from_two_sources_merge_newest_first(self):
        store.replace_filings("SHOP", "SEDAR+", [item("SEDAR+", i=1), item("SEDAR+", i=3)])
        store.replace_filings("SHOP", "SEC", [item("SEC", i=2), item("SEC", i=4)])
        rows = store.filings("SHOP")
        self.assertEqual(len(rows), 4)
        self.assertEqual([r["date"] for r in rows], sorted([r["date"] for r in rows], reverse=True))
        self.assertEqual({r["source"] for r in rows}, {"SEDAR+", "SEC"})

    def test_replacing_one_source_leaves_the_other(self):
        store.replace_filings("SHOP", "SEDAR+", [item("SEDAR+", i=1), item("SEDAR+", i=2)])
        store.replace_filings("SHOP", "SEC", [item("SEC", i=1)])
        store.replace_filings("SHOP", "SEDAR+", [item("SEDAR+", i=9)])   # refresh SEDAR+ only
        rows = store.filings("SHOP")
        self.assertEqual(sorted(r["source"] for r in rows), ["SEC", "SEDAR+"])
        self.assertEqual(len([r for r in rows if r["source"] == "SEDAR+"]), 1, "SEDAR+ replaced, not appended")
        self.assertEqual(len([r for r in rows if r["source"] == "SEC"]), 1, "SEC untouched")

    def test_a_single_row_is_fetchable_by_id_for_download(self):
        store.replace_filings("SHOP", "SEC", [item("SEC", i=7)])
        row = store.filing("SHOP", "sec:7")
        self.assertIsNotNone(row)
        self.assertEqual(row["source"], "SEC")
        self.assertTrue(row["url"].startswith("https://www.sec.gov/"))
        self.assertIsNone(store.filing("SHOP", "sec:999"))

    def test_symbols_do_not_bleed_and_the_profile_is_remembered(self):
        store.replace_filings("SHOP", "SEDAR+", [item("SEDAR+", i=1)])
        store.replace_filings("ATD", "SEDAR+", [item("SEDAR+", i=1), item("SEDAR+", i=2)])
        store.mark_filings_fetched("ATD", "000012345")
        self.assertEqual(len(store.filings("SHOP")), 1)
        self.assertEqual(len(store.filings("ATD")), 2)
        self.assertEqual(store.sedar_profile("ATD"), "000012345")

    def test_forget_clears_rows_and_stamps(self):
        store.replace_filings("SHOP", "SEC", [item("SEC", i=1)])
        store.mark_filings_fetched("SHOP", "000037100")
        store.forget_filings("SHOP")
        self.assertEqual(store.filings("SHOP"), [])
        self.assertEqual(store.filings_fetched_at("SHOP"), "")
        self.assertEqual(store.sedar_profile("SHOP"), "")

    def test_data_summary_counts_filings(self):
        store.replace_filings("SHOP", "SEC", [item("SEC", i=1), item("SEC", i=2)])
        store.replace_filings("SHOP", "SEDAR+", [item("SEDAR+", i=1)])
        self.assertEqual(store.data_summary()["filings"], 3)


class FilingsPayloadTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        self._fetch = disclosures.fetch

    def tearDown(self):
        disclosures.fetch = self._fetch
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def stub(self, items, sources):
        disclosures.fetch = lambda symbol, name="", exchange="", currency="", limit=200: {"items": items, "sources": sources}

    def test_stale_until_a_fetch_then_fresh_within_a_day(self):
        self.assertTrue(bagholder._filings_stale("SHOP"))
        store.mark_filings_fetched("SHOP")
        self.assertFalse(bagholder._filings_stale("SHOP"))

    def test_a_day_old_stamp_is_stale(self):
        old = (datetime.now(timezone.utc) - timedelta(hours=25)).strftime("%Y-%m-%dT%H:%M:%SZ")
        store.mark_filings_fetched("SHOP", now=old)
        self.assertTrue(bagholder._filings_stale("SHOP"))

    def test_a_read_no_source_answered_is_not_stamped(self):
        self.stub([], {"SEDAR+": {"available": False, "matched": False, "count": 0,
                                  "error": "the SEDAR+ bot gate turned the request away"},
                       "SEC": {"available": False, "matched": False, "count": 0, "error": ""}})
        out = bagholder.filings_payload("CH", refresh=True)
        self.assertTrue(out["sourceUnavailable"])
        self.assertEqual(store.filings_fetched_at("CH"), "", "a refusal is never stamped as a read")
        self.assertTrue(bagholder._filings_stale("CH"), "the next ask reads again")

    def test_a_read_a_source_answered_is_stamped(self):
        self.stub([item("SEC", i=1)], {"SEC": {"available": True, "matched": True, "count": 1, "error": ""}})
        bagholder.filings_payload("CH", refresh=True)
        self.assertNotEqual(store.filings_fetched_at("CH"), "")
        self.assertFalse(bagholder._filings_stale("CH"))

    def test_refresh_merges_sources_and_reports_status(self):
        self.stub(
            [item("SEDAR+", i=1, profile="000037100"), item("SEC", i=2)],
            {"SEDAR+": {"available": True, "matched": True, "count": 1, "error": ""},
             "SEC": {"available": True, "matched": True, "count": 1, "error": ""}},
        )
        out = bagholder.filings_payload("SHOP", refresh=True)
        self.assertTrue(out["ok"])
        self.assertTrue(out["available"])
        self.assertTrue(out["refreshed"])
        self.assertEqual(len(out["filings"]), 2)
        self.assertEqual(out["profileNo"], "000037100", "the SEDAR+ profile is remembered from the items")
        self.assertEqual(set(out["sources"]), {"SEDAR+", "SEC"})
        self.assertIn("Financials", out["categories"])

    def test_only_one_source_matches(self):
        self.stub(
            [item("SEC", i=1)],
            {"SEDAR+": {"available": True, "matched": False, "count": 0, "error": ""},
             "SEC": {"available": True, "matched": True, "count": 1, "error": ""}},
        )
        out = bagholder.filings_payload("NVDA", refresh=True)
        self.assertEqual([r["source"] for r in out["filings"]], ["SEC"])
        self.assertTrue(out["sources"]["SEC"]["matched"])
        self.assertFalse(out["sources"]["SEDAR+"]["matched"])

    def test_all_sources_unreachable_is_reported(self):
        self.stub([], {"SEDAR+": {"available": False, "matched": False, "count": 0, "error": "curl_cffi missing"},
                       "SEC": {"available": False, "matched": False, "count": 0, "error": "network"}})
        out = bagholder.filings_payload("SHOP", refresh=True)
        self.assertTrue(out["ok"], "the endpoint still answers cleanly")
        self.assertTrue(out["sourceUnavailable"])
        self.assertEqual(out["filings"], [])

    def test_empty_symbol_is_rejected(self):
        self.assertFalse(bagholder.filings_payload("")["ok"])


if __name__ == "__main__":
    unittest.main()


class EnrichTest(unittest.TestCase):
    """When a document is read again. A title and a one-sentence summary come from the
    same read but not always in the same pass, so a row holding one of them is not done."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=1)])
        self.doc = "sedar:1"

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def stored(self):
        row = store.filing("QNC", self.doc)
        return row.get("subject") or "", row.get("summary") or "", row.get("enrichVersion") or 0

    def enrich(self, model=True, read=("A title", "A sentence."), available=True, final=False):
        with mock.patch.object(bagholder.enrich, "summary_available", return_value=model), \
             mock.patch.object(bagholder.enrich, "summary_status", return_value="ready" if model else "off"), \
             mock.patch.object(bagholder.disclosures, "available", return_value=available), \
             mock.patch.object(bagholder.disclosures, "enrichment", return_value=None), \
             mock.patch.object(bagholder.disclosures, "content", return_value=(b"%PDF-1.4 body", "application/pdf")) as content, \
             mock.patch.object(bagholder.enrich, "enrich_document", return_value=dict({"subject": read[0], "summary": read[1]}, **({"final": True} if final else {}))):
            out = bagholder.filings_enrich("QNC", self.doc)
            return out, content.call_count

    def test_a_row_with_both_halves_is_not_read_again(self):
        self.enrich()
        self.assertEqual(self.stored()[:2], ("A title", "A sentence."))
        out, reads = self.enrich(read=("other", "other."))
        self.assertEqual(reads, 0)                       # the document is not fetched a second time
        self.assertEqual(out["subject"], "A title")

    def test_a_document_read_for_good_is_never_fetched_again_even_with_nothing_to_show(self):
        # a regulator's form is read from its own boxes and no model adds to it, so a form that
        # yields nothing is done. Without this it had neither half, counted as unfinished, and was
        # fetched from the regulator again on every ask — the page asks whenever the card is shown.
        out, reads = self.enrich(read=("", ""), final=True)
        self.assertEqual((out["subject"], out["summary"]), ("", ""))
        self.assertEqual(self.stored()[2], bagholder.ENRICH_VERSION, "stamped, so the row is done")
        for _ in range(3):
            out, reads = self.enrich(read=("", ""), final=True)
            self.assertEqual(reads, 0, "the document is not fetched again")

    def test_a_document_read_for_good_while_no_model_was_up_is_still_done(self):
        out, reads = self.enrich(model=False, read=("Exempt distribution of $1,500,000", "$1,500,000 distributed."), final=True)
        self.assertEqual(self.stored()[:2], ("Exempt distribution of $1,500,000", "$1,500,000 distributed."))
        _, reads = self.enrich(model=True, read=("other", "other."), final=True)
        self.assertEqual(reads, 0, "a form needs no model, so a model arriving later changes nothing")

    def test_a_row_holding_only_a_title_is_read_again_for_its_summary(self):
        self.enrich(read=("A title", ""))
        self.assertEqual(self.stored()[:2], ("A title", ""))
        out, reads = self.enrich(read=("A title", "The sentence."))
        self.assertEqual(reads, 1)
        self.assertEqual(out["summary"], "The sentence.")

    def test_a_row_holding_only_a_summary_is_read_again_for_its_title(self):
        self.enrich(read=("", "A sentence."))
        self.assertEqual(self.stored()[:2], ("", "A sentence."))
        out, reads = self.enrich(read=("The title", "A sentence."))
        self.assertEqual(reads, 1)
        self.assertEqual(out["subject"], "The title")

    def test_reading_again_fills_what_is_missing_and_empties_nothing(self):
        self.enrich(read=("A title", ""))
        out, reads = self.enrich(read=("", ""))          # this read found nothing at all
        self.assertEqual(reads, 1)
        self.assertEqual(out["subject"], "A title")      # what was already there survives
        self.assertEqual(self.stored()[0], "A title")

    def test_the_first_read_under_the_current_logic_still_clears_an_older_junk_title(self):
        store.set_filing_enrichment("QNC", self.doc, subject="00012345.pdf", summary="", version=1)
        out, reads = self.enrich(read=("", "A sentence."))
        self.assertEqual(reads, 1)
        self.assertEqual(out["subject"], "")             # the stale title goes rather than sticking
        self.assertEqual(out["summary"], "A sentence.")

    def test_with_no_model_up_a_row_already_read_is_not_fetched_again(self):
        self.enrich(read=("A title", ""))
        out, reads = self.enrich(model=False, read=("A title", "never asked"))
        self.assertEqual(reads, 0)                       # no summary is coming: nothing to gain by reading
        self.assertEqual(out["summary"], "")

    def test_a_row_never_read_is_read_even_with_no_model(self):
        out, reads = self.enrich(model=False, read=("A title", ""))
        self.assertEqual(reads, 1)                       # the document's own title is still worth having
        self.assertEqual(out["subject"], "A title")
        self.assertEqual(self.stored()[2], 0)            # not finalised: it is read again once a model is up


class StatusSummaryTest(unittest.TestCase):
    """The page holds a row that had no model to ask, and the status it already polls is
    what tells it one is up. Reading that must never start a model by itself."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        os.environ["BAGHOLDER_HOME"] = self.tmp.name
        store.set_home(self.tmp.name)
        bagholder.set_home(self.tmp.name)
        store.ensure()

    def tearDown(self):
        self.tmp.cleanup()
        os.environ.pop("BAGHOLDER_HOME", None)

    def test_the_status_says_whether_a_summary_could_be_made_now(self):
        with mock.patch.object(bagholder.enrich, "summary_status", return_value="ready"):
            self.assertTrue(bagholder.status_payload()["summaryReady"])
        for phase in ("off", "detecting", "downloading", "starting", "failed"):
            with mock.patch.object(bagholder.enrich, "summary_status", return_value=phase):
                self.assertFalse(bagholder.status_payload()["summaryReady"], phase)

    def test_asking_the_status_never_starts_a_model(self):
        with mock.patch.object(bagholder.enrich, "summary_status", return_value="off"), \
             mock.patch.object(bagholder.enrich, "summary_available", side_effect=AssertionError("a status poll started a model")):
            self.assertFalse(bagholder.status_payload()["summaryReady"])


class WaitingForTheModelTest(unittest.TestCase):
    """The first document read in a session is the read that starts the model. Spending it
    and coming back later is what left the newest filing without a summary."""

    setUp, tearDown, stored = EnrichTest.setUp, EnrichTest.tearDown, EnrichTest.stored

    def test_a_model_that_is_starting_is_waited_for_rather_than_the_read_wasted(self):
        with mock.patch.object(bagholder.enrich, "summary_available", return_value=False), \
             mock.patch.object(bagholder.enrich, "wait_for_summary", return_value=True) as waited, \
             mock.patch.object(bagholder.enrich, "summary_status", return_value="ready"), \
             mock.patch.object(bagholder.disclosures, "available", return_value=True), \
             mock.patch.object(bagholder.disclosures, "enrichment", return_value=None), \
             mock.patch.object(bagholder.disclosures, "content", return_value=(b"%PDF-1.4 body", "application/pdf")), \
             mock.patch.object(bagholder.enrich, "enrich_document", return_value={"subject": "A title", "summary": "A sentence."}):
            out = bagholder.filings_enrich("QNC", self.doc)
        self.assertEqual(waited.call_count, 1)
        self.assertEqual(out["summary"], "A sentence.")
        self.assertEqual(self.stored(), ("A title", "A sentence.", bagholder.ENRICH_VERSION))

    def test_a_model_that_never_comes_up_leaves_the_row_to_be_read_again(self):
        with mock.patch.object(bagholder.enrich, "summary_available", return_value=False), \
             mock.patch.object(bagholder.enrich, "wait_for_summary", return_value=False), \
             mock.patch.object(bagholder.enrich, "summary_status", return_value="off"), \
             mock.patch.object(bagholder.disclosures, "available", return_value=True), \
             mock.patch.object(bagholder.disclosures, "enrichment", return_value=None), \
             mock.patch.object(bagholder.disclosures, "content", return_value=(b"%PDF-1.4 body", "application/pdf")), \
             mock.patch.object(bagholder.enrich, "enrich_document", return_value={"subject": "A title", "summary": ""}):
            out = bagholder.filings_enrich("QNC", self.doc)
        self.assertEqual(out["subject"], "A title")
        self.assertEqual(self.stored()[2], 0)      # not finalised: read again once a model is up


class RefreshKeepsWhatWasReadTest(unittest.TestCase):
    """The list of filings is refreshed far more often than a filed document changes, so a
    refresh must not throw away what was read from the documents."""

    setUp, tearDown = EnrichTest.setUp, EnrichTest.tearDown

    def test_a_row_the_source_still_lists_keeps_its_subject_and_summary(self):
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=1), item("SEDAR+", i=2)])
        store.set_filing_enrichment("QNC", "sedar:1", subject="A title", summary="A sentence.", version=9)
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=1), item("SEDAR+", i=2), item("SEDAR+", i=3)])
        row = store.filing("QNC", "sedar:1")
        self.assertEqual((row["subject"], row["summary"], row["enrichVersion"]), ("A title", "A sentence.", 9))

    def test_what_the_source_says_about_a_row_is_still_refreshed(self):
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=1)])
        store.set_filing_enrichment("QNC", "sedar:1", subject="A title", summary="A sentence.", version=9)
        moved = item("SEDAR+", i=1)
        moved["url"] = "https://www.sedarplus.ca/x?drmKey=fresh"     # SEDAR+ mints a new link each visit
        store.replace_filings("QNC", "SEDAR+", [moved])
        row = store.filing("QNC", "sedar:1")
        self.assertEqual(row["url"], "https://www.sedarplus.ca/x?drmKey=fresh")
        self.assertEqual(row["summary"], "A sentence.")

    def test_a_row_the_source_no_longer_lists_goes(self):
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=1), item("SEDAR+", i=2)])
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=2)])
        self.assertIsNone(store.filing("QNC", "sedar:1"))
        self.assertIsNotNone(store.filing("QNC", "sedar:2"))

    def test_another_sources_rows_are_untouched(self):
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=1)])
        store.replace_filings("QNC", "SEC", [item("SEC", i=1)])
        store.set_filing_enrichment("QNC", "sec:1", subject="From EDGAR", summary="A sentence.", version=9)
        store.replace_filings("QNC", "SEDAR+", [item("SEDAR+", i=1)])
        self.assertEqual(store.filing("QNC", "sec:1")["subject"], "From EDGAR")
