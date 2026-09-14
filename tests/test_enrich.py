"""Reading a filing: subject extraction from PDF metadata, text extraction, and the
graceful behaviour when the local model or pdftotext is absent. No network and no
subprocess success is required; the pieces are exercised in isolation."""
from __future__ import annotations

import unittest

import enrich
import pdftext


_SAVED_PDF_DISABLED = None


def setUpModule():
    global _SAVED_PDF_DISABLED
    _SAVED_PDF_DISABLED = pdftext.DISABLED
    pdftext.DISABLED = True   # never install pdfminer or shell out during tests


def tearDownModule():
    pdftext.DISABLED = _SAVED_PDF_DISABLED


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

    def test_pdf_text_is_empty_when_no_engine(self):
        # with the PDF engine disabled (as in tests) a PDF yields no text, never an error
        self.assertEqual(enrich.pdf_text(b"%PDF-1.7 ..."), "")

    def test_document_text_routes_by_type(self):
        self.assertEqual(enrich.document_text(b"<p>hello there</p>", "text/html"), "hello there")


class SummaryTest(unittest.TestCase):
    def setUp(self):
        import localmodel
        self.lm = localmodel
        self._chat = localmodel.chat

    def tearDown(self):
        self.lm.chat = self._chat

    def test_no_model_means_no_summary(self):
        self.lm.chat = lambda prompt, max_tokens=90: ""   # the model is not up
        self.assertEqual(enrich.summarize("Some filing text"), "")

    def test_summary_of_empty_text_is_empty(self):
        self.assertEqual(enrich.summarize(""), "")

    def test_summary_is_kept_to_one_sentence(self):
        self.lm.chat = lambda prompt, max_tokens=90: "It announces a private placement. Extra sentence."
        self.assertEqual(enrich.summarize("text"), "It announces a private placement.")

    def test_summary_strips_chat_template_tokens(self):
        self.lm.chat = lambda prompt, max_tokens=90: "It announces a private placement.<|eot_id|>"
        self.assertEqual(enrich.summarize("text"), "It announces a private placement.")

    def test_title_strips_a_chatty_preamble_and_markdown(self):
        self.lm.chat = lambda prompt, max_tokens=90: "Sure, here is the title: **Closing of $1.5M Drawdown**"
        self.assertEqual(enrich.title_from_model("some filing text"), "Closing of $1.5M Drawdown")

    def test_title_rejects_a_bare_form_code_or_echo(self):
        self.lm.chat = lambda prompt, max_tokens=90: "Schedule 13G"
        self.assertEqual(enrich.title_from_model("text"), "")   # too short to beat the type already shown
        self.lm.chat = lambda prompt, max_tokens=90: "Here is a title for the filing"
        self.assertEqual(enrich.title_from_model("text"), "")

    def test_no_model_means_no_title(self):
        self.lm.chat = lambda prompt, max_tokens=90: ""
        self.assertEqual(enrich.title_from_model("text"), "")

    def test_enrich_document_titles_from_the_model_when_there_is_no_pdf_subject(self):
        # HTML (SEC) has no PDF metadata subject, so the title comes from the model
        def chat(prompt, max_tokens=90):
            return "Q2 2026 MD&A and interim financial statements" if "Title:" in prompt else "It reports Q2 2026 results."
        self.lm.chat = chat
        info = enrich.enrich_document("SEC", b"<html><body>Management discussion...</body></html>", "text/html")
        self.assertEqual(info["subject"], "Q2 2026 MD&A and interim financial statements")
        self.assertEqual(info["summary"], "It reports Q2 2026 results.")

    def test_enrich_document_gives_subject_without_a_model(self):
        self.lm.chat = lambda prompt, max_tokens=90: ""
        info = enrich.enrich_document("SEDAR+", pdf_with_title(b"Microsoft Word - Acme Announces Buyback EN"), "application/pdf")
        self.assertEqual(info["subject"], "Acme Announces Buyback")
        self.assertEqual(info["summary"], "")


if __name__ == "__main__":
    unittest.main()
