package app

import (
	"strings"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func TestAFormWithNoTitleReadYetIsNamedInWords(t *testing.T) {
	for code, want := range map[string]string{"4": "Insider transaction (Form 4)", "8-k": "Material event (8-K)", "ZZZ": "ZZZ"} {
		if got := formName(code); got != want {
			t.Errorf("formName(%q) = %q, want %q", code, got, want)
		}
	}
}

func TestANoticeCarriesWhenTheThingHappened(t *testing.T) {
	rows := []store.WireItem{
		{ID: "a", PublishedAt: "2026-07-01T12:00:00Z", URL: "https://one.example"},
		{ID: "b", PublishedAt: "2026-09-18T09:00:00Z", URL: "https://two.example"},
	}
	extra := noticeExtra("QNC", "TSX-V", wireNoticeRows(rows))
	if extra["at"] != "2026-09-18T09:00:00Z" {
		t.Errorf("at = %v, want the newest row's own moment", extra["at"])
	}
	if extra["url"] != "https://two.example" {
		t.Errorf("url = %v, want the newest row's page", extra["url"])
	}
	if extra["exchange"] != "TSX-V" {
		t.Errorf("exchange = %v", extra["exchange"])
	}
}

func TestAFiledDocumentOpensThroughTheApp(t *testing.T) {
	rows := []store.Filing{{ID: "sedar:9", Source: "SEDAR+", Date: "2026-09-10", URL: "https://sedarplus.ca/x"}}
	extra := noticeExtra("CH", "", filingNoticeRows(rows))
	if extra["doc"] != "sedar:9" || extra["source"] != "SEDAR+" {
		t.Errorf("a filed document did not carry its reader: %v", extra)
	}
	wire := noticeExtra("QNC", "", wireNoticeRows([]store.WireItem{{ID: "gnw:1", Source: "GlobeNewswire", PublishedAt: "2026-09-10T00:00:00Z", URL: "https://wire.example"}}))
	if _, ok := wire["doc"]; ok {
		t.Errorf("a wire item was given a document reader: %v", wire)
	}
	if wire["url"] != "https://wire.example" {
		t.Errorf("url = %v", wire["url"])
	}
}

func TestAPerShareAmountReadsAsTheRecordStatesIt(t *testing.T) {
	for _, c := range []struct {
		amount   float64
		currency string
		want     string
	}{{0.1489, "CAD", "$0.1489"}, {0.15, "CAD", "$0.15"}, {1, "CAD", "$1.00"}, {0.25, "USD", "US$0.25"}} {
		if got := moneyPerShare(c.amount, c.currency); got != c.want {
			t.Errorf("moneyPerShare(%v, %q) = %q, want %q", c.amount, c.currency, got, c.want)
		}
	}
}

func TestADistributionReleaseCarriesWhatItPays(t *testing.T) {
	a := releaseApp(t)
	a.st.UpsertDistributions("QNC", []store.Distribution{
		{ExDate: "2026-08-29", PayDate: "2026-09-15", Amount: 0.1489, Currency: "CAD"},
		{ExDate: "2026-07-31", PayDate: "2026-08-15", Amount: 0.1300, Currency: "CAD"},
	}, "test")
	detail := a.distributionDetail("QNC")
	for _, want := range []string{"$0.1489 a share", "ex Aug 29", "paid Sep 15", "was $0.13"} {
		if !strings.Contains(detail, want) {
			t.Errorf("distribution detail %q lacks %q", detail, want)
		}
	}
	_, body := a.releaseNoticeWire("QNC", []store.WireItem{wire("tmx:1", "Aegis Announces August 2026 Distributions", "2026-09-18T12:00:00Z")})
	if !strings.Contains(body, "$0.1489 a share") {
		t.Errorf("a distribution release said nothing a holder can act on: %q", body)
	}
	_, plain := a.releaseNoticeWire("QNC", []store.WireItem{wire("tmx:2", "Aegis names a new director", "2026-09-18T12:00:00Z")})
	if strings.Contains(plain, "a share") {
		t.Errorf("an ordinary release carried distribution figures: %q", plain)
	}
}

func TestARefusedDocumentReadsAsWords(t *testing.T) {
	a := newTestApp(t)
	page := string(a.documentErrorPage("CH", "sedar:9", "the SEDAR+ bot gate turned the request away"))
	for _, want := range []string{"would not serve this document", "bot gate", "Try again", "/api/filings/doc?symbol=CH"} {
		if !strings.Contains(page, want) {
			t.Errorf("the refusal page lacks %q", want)
		}
	}
}

func TestAReleaseCarriesWhatItsSourceSaidBeneathTheHeadline(t *testing.T) {
	a := releaseApp(t)
	row := wire("tmx:1", "Aegis names a new director", "2026-09-18T12:00:00Z")
	row.Summary = "The board appointed Jane Roe, formerly of the exchange, effective immediately."
	_, body := a.releaseNoticeWire("QNC", []store.WireItem{row})
	if !strings.Contains(body, "Jane Roe") {
		t.Errorf("the notice dropped what the source said: %q", body)
	}
	bare := wire("tmx:2", "Aegis names a new director", "2026-09-18T12:00:00Z")
	if _, plain := a.releaseNoticeWire("QNC", []store.WireItem{bare}); strings.Contains(plain, "\n") {
		t.Errorf("a release with no summary carried a second line: %q", plain)
	}
}
