package enrich

import "testing"

func noSystemPDFToText(t *testing.T) {
	t.Setenv("BAGHOLDER_NO_PDF", "")
	t.Setenv("PATH", t.TempDir())
}

func TestDisabledReturnsEmptyAndNeverProvisions(t *testing.T) {
	noPDF(t)
	if got := PDFText([]byte("%PDF-1.7 hello")); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestEnsureIsANoopWhenDisabled(t *testing.T) {
	t.Skip("pip provisioning (pdftext.ensure) has no Go counterpart: the Go extractor is compiled in")
}

func TestNonPDFBytesYieldNoText(t *testing.T) {
	noSystemPDFToText(t)
	if got := PDFText([]byte("<html>not a pdf</html>")); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestUsesTheExtractorWhenAvailable(t *testing.T) {
	noSystemPDFToText(t)
	if got := PDFText(realPDF(nil, "Extracted body text.")); got != "Extracted body text." {
		t.Errorf("got %q, want %q", got, "Extracted body text.")
	}
}

func TestABrokenExtractorIsSwallowed(t *testing.T) {
	noSystemPDFToText(t)
	if got := PDFText([]byte("%PDF-1.7 ...")); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}
