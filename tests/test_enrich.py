"""Reading a filing: subject extraction from PDF metadata, text extraction, and the
graceful behaviour when the local model or pdftotext is absent. No network and no
subprocess success is required; the pieces are exercised in isolation."""
from __future__ import annotations

import unittest

import enrich


def pdf_with_title(title_bytes):
    return b"%PDF-1.7\n1 0 obj<< /Title (" + title_bytes + b") >>\nendobj\n%%EOF"


class SubjectTest(unittest.TestCase):
    def test_a_word_authored_title_reduces_to_its_subject(self):
        data = pdf_with_title(b"Microsoft Word - CHARBONE - Closing 2nd Drawdown PR_FINAL_EN_2026-09-04_v6")
        self.assertEqual(enrich.extract_pdf_subject(data), "CHARBONE - Closing 2nd Drawdown")

    def test_language_version_and_date_tails_are_stripped(self):
        self.assertEqual(enrich._clean_subject("Q3 2026 Results PR EN v3"), "Q3 2026 Results")
        self.assertEqual(enrich._clean_subject("Prospectus Supplement No 3 FINAL"), "Prospectus Supplement No 3")

    def test_a_generic_or_empty_title_yields_no_subject(self):
        self.assertEqual(enrich.extract_pdf_subject(pdf_with_title(b"News release")), "")
        self.assertEqual(enrich.extract_pdf_subject(pdf_with_title(b"Document")), "")
        self.assertEqual(enrich.extract_pdf_subject(b"%PDF-1.7 no title here"), "")

    def test_a_utf16_hex_title_is_decoded(self):
        # "Financing" in UTF-16BE with BOM, hex-encoded as PDFs often store it
        raw = ("feff" + "".join("%04x" % ord(c) for c in "Financing Update")).encode()
        data = b"%PDF-1.7\n1 0 obj<< /Title <" + raw + b"> >>\nendobj"
        self.assertEqual(enrich.extract_pdf_subject(data), "Financing Update")

    def test_non_pdf_bytes_have_no_pdf_subject(self):
        self.assertEqual(enrich.extract_pdf_subject(b"<html>...</html>"), "")


class TextTest(unittest.TestCase):
    def test_html_text_drops_scripts_and_tags(self):
        html = b"<html><head><style>.x{}</style></head><body><h1>Results</h1><script>x()</script><p>Net income up 20%</p></body></html>"
        self.assertEqual(enrich.html_text(html), "Results Net income up 20%")

    def test_pdf_text_is_empty_without_pdftotext(self):
        import shutil
        saved = shutil.which
        try:
            shutil.which = lambda name: None
            self.assertEqual(enrich.pdf_text(b"%PDF-1.7 ..."), "")
        finally:
            shutil.which = saved

    def test_document_text_routes_by_type(self):
        self.assertEqual(enrich.document_text(b"<p>hello there</p>", "text/html"), "hello there")


class SummaryTest(unittest.TestCase):
    def test_summarize_returns_empty_when_no_model_answers(self):
        saved = enrich.OLLAMA_URL
        try:
            enrich.OLLAMA_URL = "http://127.0.0.1:1"   # nothing listens here
            self.assertEqual(enrich.summarize("Some filing text"), "")
            self.assertFalse(enrich.summary_available())
        finally:
            enrich.OLLAMA_URL = saved

    def test_summarize_of_empty_text_is_empty(self):
        self.assertEqual(enrich.summarize(""), "")

    def test_summarize_keeps_one_sentence(self):
        # stub the model call (enrich bound urlopen by name) to return a multi-sentence blob
        saved = enrich.urlopen

        class FakeResp:
            def read(self_):
                import json
                return json.dumps({"response": "It announces a private placement. Extra sentence."}).encode()

        try:
            enrich.urlopen = lambda *a, **k: FakeResp()
            self.assertEqual(enrich.summarize("text"), "It announces a private placement.")
        finally:
            enrich.urlopen = saved

    def test_enrich_document_gives_subject_without_a_model(self):
        data = pdf_with_title(b"Microsoft Word - Acme Announces Buyback EN")
        info = enrich.enrich_document("SEDAR+", data, "application/pdf")
        self.assertEqual(info["subject"], "Acme Announces Buyback")
        self.assertEqual(info["summary"], "")   # no model running in the test


if __name__ == "__main__":
    unittest.main()
