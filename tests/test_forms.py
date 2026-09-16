"""Filings whose facts sit on the page in a fixed shape: read exactly, never summarized."""
from __future__ import annotations

import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import enrich  # noqa: E402
import forms  # noqa: E402

# the shape CHARBONE's report of exempt distribution actually extracts as, instructions and all
F1 = ("Form 45-106F1 Report of Exempt Distribution ITEM 1 - REPORT TYPE New report Amended report If amended, provide "
      "filing date of report that is being amended. (YYYY-MM-DD) ITEM 2 - PARTY CERTIFYING THE REPORT Indicate the party "
      "certifying the report (select only one). For guidance regarding whether an issuer is an investment fund, refer to "
      "section 1.1 of National Instrument 81-106 Investment Fund Continuous Disclosure. ITEM 3 - ISSUER NAME Charbone "
      "Corporation ITEM 7 - INFORMATION ABOUT THE DISTRIBUTION b) Distribution dates State the distribution start and end "
      "dates. Start date 2026 YYYY 09 08 MM DD End date 2026 YYYY 09 08 MM DD c) Detailed purchaser information Complete "
      "Schedule 1 of this form for each purchaser. Province or country Exemption relied on Number of unique purchasers "
      "Total amount (Canadian $) Gibraltar NI 45-106 2.10 [Minimum amount investment] 1 1,500,000.0000 "
      "Total dollar amount of securities distributed $1,500,000.0000 Total number of unique 1")

RELEASE = ("CHARBONE Corporation announces the closing of a second drawdown of $1.5M with RiverFort Global Opportunities "
           "PCC Ltd. The proceeds will accelerate the growth of its industrial gas platform across North America, the "
           "company said on Tuesday. Management will host a call to discuss the transaction.")


class FormTest(unittest.TestCase):

    def test_a_regulators_fill_in_form_is_told_from_something_written(self):
        self.assertTrue(forms.is_form(F1))
        self.assertFalse(forms.is_form(RELEASE), "a news release is written, not filled in")
        self.assertFalse(forms.is_form(""))

    def test_the_exempt_distribution_report_is_read_value_by_value(self):
        out = forms.read(F1)
        self.assertEqual(out["subject"], "Exempt distribution of $1,500,000")
        self.assertEqual(out["summary"], "$1,500,000 distributed from 1 purchaser on 8 September 2026, "
                                         "under NI 45-106 2.10 (minimum amount investment).")

    def test_a_value_the_form_does_not_carry_is_left_out_rather_than_filled_in(self):
        thin = "Form 45-106F1 Report of Exempt Distribution Total dollar amount of securities distributed $250,000.0000"
        self.assertEqual(forms.read(thin)["summary"], "$250,000 distributed.")
        self.assertEqual(forms.read("Form 45-106F1 Report of Exempt Distribution and nothing else"), {},
                         "a form with no values read is not read at all")

    def test_several_purchasers_read_as_several(self):
        many = F1.replace("Total number of unique 1", "Total number of unique 14")
        self.assertIn("from 14 purchasers", forms.read(many)["summary"])

    def test_nothing_else_is_claimed(self):
        self.assertEqual(forms.read(RELEASE), {})


class NeverGuessTest(unittest.TestCase):
    """The rules that keep a filing from being described by a model that did not find out."""

    def test_a_hedged_line_is_thrown_away(self):
        for guess in ("The company announces the completion of a new report, likely a Form 45-106F1.",
                      "This appears to be a report of exempt distribution.",
                      "The filing may be related to a private placement.",
                      "It is not clear what the document reports."):
            self.assertTrue(enrich.hedged(guess), guess)
        self.assertFalse(enrich.hedged("The company closed a $1.5M drawdown with RiverFort."))

    def test_a_model_that_hedges_yields_no_summary_and_no_title(self):
        with mock.patch.object(enrich.localmodel, "available", return_value=True), \
             mock.patch.object(enrich.localmodel, "chat", return_value="The filing likely reports a distribution of securities."):
            self.assertEqual(enrich.summarize("Some filing text."), "")
            self.assertEqual(enrich.title_from_model("Some filing text."), "")

    def test_a_form_this_app_cannot_read_is_not_summarized_at_all(self):
        unknown = ("Form 51-999F9 Something New (YYYY-MM-DD) refer to Part B of the Instructions. Complete Schedule 2 "
                   "for each holder. Select only one. If applicable, provide the filing date.")
        with mock.patch.object(enrich, "document_text", return_value=unknown), \
             mock.patch.object(enrich.localmodel, "available", return_value=True), \
             mock.patch.object(enrich.localmodel, "chat", side_effect=AssertionError("asked a model about a form")):
            out = enrich.enrich_document("sedar", b"%PDF-1.7 no title", "application/pdf")
        self.assertEqual(out, {"subject": "", "summary": "", "final": True},
                         "the row's own type says what it is, and the read is done: no half is coming")

    def test_a_form_this_app_reads_never_sees_a_model(self):
        with mock.patch.object(enrich, "document_text", return_value=F1), \
             mock.patch.object(enrich.localmodel, "chat", side_effect=AssertionError("asked a model about a form")):
            out = enrich.enrich_document("sedar", b"%PDF-1.7 no title", "application/pdf")
        self.assertEqual(out["subject"], "Exempt distribution of $1,500,000")
        self.assertIn("1 purchaser", out["summary"])

    def test_something_written_is_still_summarized(self):
        with mock.patch.object(enrich, "document_text", return_value=RELEASE), \
             mock.patch.object(enrich.localmodel, "available", return_value=True), \
             mock.patch.object(enrich.localmodel, "chat", return_value="CHARBONE closed a $1.5M drawdown with RiverFort."):
            out = enrich.enrich_document("sedar", b"<html>release</html>", "text/html")
        self.assertEqual(out["summary"], "CHARBONE closed a $1.5M drawdown with RiverFort.")


if __name__ == "__main__":
    unittest.main()
