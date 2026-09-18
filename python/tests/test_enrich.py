"""Reading a filing: subject extraction from PDF metadata, text extraction, and the
graceful behaviour when the local model or pdftotext is absent. No network and no
subprocess success is required; the pieces are exercised in isolation."""
from __future__ import annotations

import unittest
from unittest import mock

import enrich
import formnames
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

    def test_a_title_of_bytes_that_merely_decoded_is_no_title(self):
        # what CHARBONE's report of exempt distribution actually stored: a PDF whose strings are
        # not text, decoded into characters. `Btu` is in there, so asking for three letters in a
        # row passed it, and the table and a notification both read it out.
        binary = b"\\022\x8a\xf0,0\x91\x9f\xbfO\xf9\xff\xaf\xe2U\xc0<w\xb0\\027\xb3c\xab\\)Q\xb1\xba \x9cO\xb4.\xb4\xf9 xBtu\x83\xe5\\f!"
        self.assertEqual(enrich.extract_pdf_subject(pdf_with_title(binary)), "")
        self.assertFalse(enrich.readable(binary.decode("latin-1")))

    def test_what_reads_as_a_title_and_what_does_not(self):
        for good in ("CHARBONE - Closing 2nd Drawdown", "D\u00e9claration de placement avec dispense 45-106F1",
                     "Q3 2026 Interim Financial Statements", "Form 45-106F1 Report of Exempt Distribution"):
            self.assertTrue(enrich.readable(good), good)
        for bad in ("2026-09-04", "\u00b1\u00ba\u00b4\u00ab\u00b9\u00b2", "", "   ", "\x0c\x12 Report", "45-106"):
            self.assertFalse(enrich.readable(bad), repr(bad))

    def test_an_unreadable_title_falls_back_to_the_model(self):
        def chat(prompt, **kw):
            return "Report of exempt distribution in Canada" if "Title:" in prompt else "It reports a distribution."
        data = pdf_with_title(b"\x8a\xf0,0\x91\x9f\xbfO\xf9\xff\xaf\xe2U\xc0<w\xb0")
        with mock.patch.object(enrich.localmodel, "available", return_value=True), \
             mock.patch.object(enrich.localmodel, "chat", side_effect=chat), \
             mock.patch.object(enrich, "document_text", return_value="A report of exempt distribution."):
            out = enrich.enrich_document("sedar", data, "application/pdf")
        self.assertEqual(out["subject"], "Report of exempt distribution in Canada")

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


class SentenceTest(unittest.TestCase):
    """Where a summary ends. A filing's summary opens by naming the issuer, and a company's
    name ends in a full stop far more often than a sentence does."""

    def test_a_company_suffix_does_not_end_the_sentence(self):
        for opening in ("Quantum eMotion Corp.", "Aegis Critical Energy Defence Corp.", "Shopify Inc.",
                        "High Tide Ltd.", "Brookfield Co.", "Barrick PLC"):
            line = opening + " announces a commercial order for seven units in the United States."
            self.assertEqual(enrich.first_sentence(line), line, opening)

    def test_an_initial_or_an_abbreviation_does_not_end_it_either(self):
        for line in ("U.S. regulators approved the base shelf prospectus.",
                     "Dr. Chen was appointed chief scientist of the company.",
                     "No. 4 of the schedule lists the securities offered.",
                     "J. Smith resigned from the board of directors."):
            self.assertEqual(enrich.first_sentence(line), line, line)

    def test_a_real_second_sentence_is_dropped(self):
        self.assertEqual(enrich.first_sentence("The company files Q1 statements. It also names a director."),
                         "The company files Q1 statements.")
        self.assertEqual(enrich.first_sentence("Is the prospectus final? The company says yes."),
                         "Is the prospectus final?")

    def test_a_stop_followed_by_more_of_the_same_sentence_is_not_an_ending(self):
        line = "The filing lists exhibits 1.2 and 3. and describes the securities offered."
        self.assertEqual(enrich.first_sentence(line), line)

    def test_an_answer_with_no_stop_at_all_survives_whole(self):
        self.assertEqual(enrich.first_sentence("Quantum eMotion Corp files its interim statements"),
                         "Quantum eMotion Corp files its interim statements")
        self.assertEqual(enrich.first_sentence(""), "")

    def test_the_summary_keeps_the_whole_sentence_rather_than_the_name_alone(self):
        said = "Quantum eMotion Corp. announces its participation as a sponsor of the AI for Good Global Summit."
        with mock.patch.object(enrich.localmodel, "chat", return_value=said):
            self.assertEqual(enrich.summarize("the filing's text"), said)

    def test_a_bare_name_is_still_no_summary(self):
        with mock.patch.object(enrich.localmodel, "chat", return_value="Quantum eMotion Corp."):
            self.assertEqual(enrich.summarize("the filing's text"), "")


class FormNamesTest(unittest.TestCase):
    def test_a_form_is_named_by_its_code(self):
        self.assertEqual(formnames.title_of("4"), "Form 4: Statement of changes in beneficial ownership")
        self.assertEqual(formnames.title_of("144"), "Form 144: Notice of proposed sale of securities")
        self.assertEqual(formnames.title_of("424B5"), "Form 424B5: Prospectus")
        self.assertEqual(formnames.title_of("S-1/A"), "Form S-1: Registration statement (amended)")
        self.assertEqual(formnames.title_of("DEF 14A"), "Form DEF 14A: Proxy statement")
        self.assertIsNone(formnames.title_of("8-K"), "a current report is named by its items")
        self.assertIsNone(formnames.title_of("45-106F1"), "not one of the SEC's own codes")

    def test_a_current_report_is_named_by_its_items(self):
        one = "Item 2.02 Results of Operations and Financial Condition. Item 9.01 Financial Statements and Exhibits."
        self.assertEqual(formnames.items_title("8-K", one), "Form 8-K: Results of operations and financial condition")
        two = "Item 5.02 Departure of Directors. Item 7.01 Regulation FD Disclosure. Item 9.01 Exhibits."
        self.assertEqual(formnames.items_title("8-K", two),
                         "Form 8-K: Departure or election of directors or officers and Regulation FD disclosure")
        many = "Item 1.01. Item 2.01. Item 3.02. Item 8.01."
        self.assertEqual(formnames.items_title("8-K", many), "Form 8-K: Entry into a material agreement and 3 other items")
        self.assertIsNone(formnames.items_title("8-K", "no items here"))
        self.assertIsNone(formnames.items_title("8-K", "Item 9.01 Financial Statements and Exhibits"),
                          "every report has exhibits")


class RestatedFormTest(unittest.TestCase):
    def test_a_summary_does_not_restate_the_form(self):
        strip = enrich._strip_preamble
        self.assertEqual(strip("This Form 8-K reports on the resale of shares by the Department of Commerce."),
                         "Resale of shares by the Department of Commerce.")
        self.assertEqual(strip("This filing contains a proposed sale of Class A common stock by Yao Huiwen."),
                         "A proposed sale of Class A common stock by Yao Huiwen.")
        self.assertEqual(strip("This news release announces a bought deal offering of 10,350,000 units."),
                         "A bought deal offering of 10,350,000 units.")
        self.assertEqual(strip("This filing contains"), "This filing contains")
        self.assertEqual(strip("Quarterly results for the third quarter."), "Quarterly results for the third quarter.")
