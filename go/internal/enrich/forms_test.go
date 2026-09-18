package enrich

import (
	"strings"
	"testing"
)

const f1 = "Form 45-106F1 Report of Exempt Distribution ITEM 1 - REPORT TYPE New report Amended report If amended, provide " +
	"filing date of report that is being amended. (YYYY-MM-DD) ITEM 2 - PARTY CERTIFYING THE REPORT Indicate the party " +
	"certifying the report (select only one). For guidance regarding whether an issuer is an investment fund, refer to " +
	"section 1.1 of National Instrument 81-106 Investment Fund Continuous Disclosure. ITEM 3 - ISSUER NAME Charbone " +
	"Corporation ITEM 7 - INFORMATION ABOUT THE DISTRIBUTION b) Distribution dates State the distribution start and end " +
	"dates. Start date 2026 YYYY 09 08 MM DD End date 2026 YYYY 09 08 MM DD c) Detailed purchaser information Complete " +
	"Schedule 1 of this form for each purchaser. Province or country Exemption relied on Number of unique purchasers " +
	"Total amount (Canadian $) Gibraltar NI 45-106 2.10 [Minimum amount investment] 1 1,500,000.0000 " +
	"Total dollar amount of securities distributed $1,500,000.0000 Total number of unique 1"

const release = "CHARBONE Corporation announces the closing of a second drawdown of $1.5M with RiverFort Global Opportunities " +
	"PCC Ltd. The proceeds will accelerate the growth of its industrial gas platform across North America, the " +
	"company said on Tuesday. Management will host a call to discuss the transaction."

func TestARegulatorsFillInFormIsToldFromSomethingWritten(t *testing.T) {
	if !IsForm(f1) {
		t.Errorf("IsForm(f1) = false")
	}
	if IsForm(release) {
		t.Errorf("a news release is written, not filled in")
	}
	if IsForm("") {
		t.Errorf("IsForm(\"\") = true")
	}
}

func TestTheExemptDistributionReportIsReadValueByValue(t *testing.T) {
	out := ReadForm(f1)
	if out == nil {
		t.Fatal("ReadForm(f1) = nil")
	}
	if out.Subject != "Exempt distribution of $1,500,000" {
		t.Errorf("got %q, want %q", out.Subject, "Exempt distribution of $1,500,000")
	}
	want := "$1,500,000 distributed from 1 purchaser on 8 September 2026, under NI 45-106 2.10 (minimum amount investment)."
	if out.Summary != want {
		t.Errorf("got %q, want %q", out.Summary, want)
	}
}

func TestAValueTheFormDoesNotCarryIsLeftOutRatherThanFilledIn(t *testing.T) {
	thin := "Form 45-106F1 Report of Exempt Distribution Total dollar amount of securities distributed $250,000.0000"
	out := ReadForm(thin)
	if out == nil {
		t.Fatal("ReadForm(thin) = nil")
	}
	if out.Summary != "$250,000 distributed." {
		t.Errorf("got %q, want %q", out.Summary, "$250,000 distributed.")
	}
	if got := ReadForm("Form 45-106F1 Report of Exempt Distribution and nothing else"); got != nil {
		t.Errorf("a form with no values read is not read at all: got %+v", *got)
	}
}

func TestSeveralPurchasersReadAsSeveral(t *testing.T) {
	many := strings.Replace(f1, "Total number of unique 1", "Total number of unique 14", 1)
	out := ReadForm(many)
	if out == nil {
		t.Fatal("ReadForm(many) = nil")
	}
	if !strings.Contains(out.Summary, "from 14 purchasers") {
		t.Errorf("%q does not contain %q", out.Summary, "from 14 purchasers")
	}
}

func TestNothingElseIsClaimed(t *testing.T) {
	if got := ReadForm(release); got != nil {
		t.Errorf("got %+v, want nil", *got)
	}
}

func TestAHedgedLineIsThrownAway(t *testing.T) {
	for _, guess := range []string{"The company announces the completion of a new report, likely a Form 45-106F1.",
		"This appears to be a report of exempt distribution.",
		"The filing may be related to a private placement.",
		"It is not clear what the document reports."} {
		if !Hedged(guess) {
			t.Errorf("%s", guess)
		}
	}
	if Hedged("The company closed a $1.5M drawdown with RiverFort.") {
		t.Errorf("hedged(%q) = true", "The company closed a $1.5M drawdown with RiverFort.")
	}
}

func TestAModelThatHedgesYieldsNoSummaryAndNoTitle(t *testing.T) {
	e := &Enricher{Model: modelSaying(t, "The filing likely reports a distribution of securities.")}
	if got := e.Summarize("Some filing text."); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
	if got := e.TitleFromModel("Some filing text."); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestAFormThisAppCannotReadIsNotSummarizedAtAll(t *testing.T) {
	t.Setenv("BAGHOLDER_NO_PDF", "")
	unknown := "Form 51-999F9 Something New (YYYY-MM-DD) refer to Part B of the Instructions. Complete Schedule 2 " +
		"for each holder. Select only one. If applicable, provide the filing date."
	e := &Enricher{Model: modelAnswering(t, func(string) string {
		t.Errorf("asked a model about a form")
		return ""
	})}
	out := e.EnrichDocument("sedar", realPDF(nil, unknown), "application/pdf")
	if out != (Result{Subject: "", Summary: "", Final: true}) {
		t.Errorf("the row's own type says what it is, and the read is done: no half is coming: got %+v", out)
	}
}

func TestAFormThisAppReadsNeverSeesAModel(t *testing.T) {
	t.Setenv("BAGHOLDER_NO_PDF", "")
	e := &Enricher{Model: modelAnswering(t, func(string) string {
		t.Errorf("asked a model about a form")
		return ""
	})}
	out := e.EnrichDocument("sedar", realPDF(nil, f1), "application/pdf")
	if out.Subject != "Exempt distribution of $1,500,000" {
		t.Errorf("got %q, want %q", out.Subject, "Exempt distribution of $1,500,000")
	}
	if !strings.Contains(out.Summary, "1 purchaser") {
		t.Errorf("%q does not contain %q", out.Summary, "1 purchaser")
	}
}

func TestSomethingWrittenIsStillSummarized(t *testing.T) {
	e := &Enricher{Model: modelSaying(t, "CHARBONE closed a $1.5M drawdown with RiverFort.")}
	out := e.EnrichDocument("sedar", []byte("<html>"+release+"</html>"), "text/html")
	if out.Summary != "CHARBONE closed a $1.5M drawdown with RiverFort." {
		t.Errorf("got %q, want %q", out.Summary, "CHARBONE closed a $1.5M drawdown with RiverFort.")
	}
}
