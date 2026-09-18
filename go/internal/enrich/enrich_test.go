package enrich

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func noPDF(t *testing.T) { t.Setenv("BAGHOLDER_NO_PDF", "1") }

func pdfWithTitle(title []byte) []byte {
	return append(append([]byte("%PDF-1.7\n1 0 obj<< /Title ("), title...), []byte(") >>\nendobj\n%%EOF")...)
}

func realPDF(title []byte, text string) []byte {
	esc := strings.NewReplacer(`\`, `\\`, `(`, `\(`, `)`, `\)`)
	content := "BT 1 0 0 1 72 720 Tm (" + esc.Replace(text) + ") Tj ET"
	objs := [][]byte{
		[]byte("<< /Type /Catalog /Pages 2 0 R >>"),
		[]byte("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
		[]byte("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>"),
		[]byte(fmt.Sprintf("<< /Length %d >>\nstream\n%s\nendstream", len(content), content)),
	}
	if title != nil {
		objs = append(objs, append(append([]byte("<< /Title ("), title...), []byte(") >>")...))
	}
	var b bytes.Buffer
	b.WriteString("%PDF-1.7\n")
	var offsets []int
	for i, o := range objs {
		offsets = append(offsets, b.Len())
		fmt.Fprintf(&b, "%d 0 obj\n", i+1)
		b.Write(o)
		b.WriteString("\nendobj\n")
	}
	xref := b.Len()
	fmt.Fprintf(&b, "xref\n0 %d\n0000000000 65535 f \n", len(objs)+1)
	for _, off := range offsets {
		fmt.Fprintf(&b, "%010d 00000 n \n", off)
	}
	fmt.Fprintf(&b, "trailer\n<< /Size %d /Root 1 0 R", len(objs)+1)
	if title != nil {
		fmt.Fprintf(&b, " /Info %d 0 R", len(objs))
	}
	fmt.Fprintf(&b, " >>\nstartxref\n%d\n%%%%EOF\n", xref)
	return b.Bytes()
}

func modelAnswering(t *testing.T, answer func(prompt string) string) *LocalModel {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			Messages []struct {
				Content string `json:"content"`
			} `json:"messages"`
		}
		json.NewDecoder(r.Body).Decode(&req)
		prompt := ""
		if len(req.Messages) > 0 {
			prompt = req.Messages[0].Content
		}
		json.NewEncoder(w).Encode(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": answer(prompt)}}}})
	}))
	t.Cleanup(srv.Close)
	lm := NewLocalModel(t.TempDir())
	lm.endpoint = srv.URL
	return lm
}

func modelSaying(t *testing.T, reply string) *LocalModel {
	return modelAnswering(t, func(string) string { return reply })
}

func noModel(t *testing.T) *LocalModel {
	srv := httptest.NewServer(http.NotFoundHandler())
	t.Cleanup(srv.Close)
	t.Setenv("BAGHOLDER_LLM_URL", "")
	t.Setenv("BAGHOLDER_OLLAMA_URL", srv.URL)
	t.Setenv("BAGHOLDER_LLAMAFILE_URL", "http://127.0.0.1:1/none.llamafile")
	return NewLocalModel(t.TempDir())
}

func modelDown(t *testing.T) *LocalModel {
	lm := noModel(t)
	lm.phase = "downloading"
	return lm
}

func TestAWordAuthoredTitleReducesToItsSubject(t *testing.T) {
	noPDF(t)
	data := pdfWithTitle([]byte("Microsoft Word - CHARBONE - Closing 2nd Drawdown PR_FINAL_EN_2026-09-04_v6"))
	if got := ExtractPDFSubject(data); got != "CHARBONE - Closing 2nd Drawdown" {
		t.Errorf("got %q, want %q", got, "CHARBONE - Closing 2nd Drawdown")
	}
}

func TestLanguageVersionAndDateTailsAreStripped(t *testing.T) {
	noPDF(t)
	if got := cleanSubject("Q3 2026 Results PR EN v3"); got != "Q3 2026 Results" {
		t.Errorf("got %q, want %q", got, "Q3 2026 Results")
	}
	if got := cleanSubject("Prospectus Supplement No 3 FINAL"); got != "Prospectus Supplement No 3" {
		t.Errorf("got %q, want %q", got, "Prospectus Supplement No 3")
	}
}

func TestAGenericOrEmptyTitleYieldsNoSubject(t *testing.T) {
	noPDF(t)
	if got := ExtractPDFSubject(pdfWithTitle([]byte("News release"))); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
	if got := ExtractPDFSubject(pdfWithTitle([]byte("Document"))); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
	if got := ExtractPDFSubject([]byte("%PDF-1.7 no title here")); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestAUTF16HexTitleIsDecoded(t *testing.T) {
	noPDF(t)
	var raw strings.Builder
	raw.WriteString("feff")
	for _, c := range "Financing Update" {
		fmt.Fprintf(&raw, "%04x", c)
	}
	data := []byte("%PDF-1.7\n1 0 obj<< /Title <" + raw.String() + "> >>\nendobj")
	if got := ExtractPDFSubject(data); got != "Financing Update" {
		t.Errorf("got %q, want %q", got, "Financing Update")
	}
}

func TestATitleOfBytesThatMerelyDecodedIsNoTitle(t *testing.T) {
	noPDF(t)
	binary := []byte("\\022\x8a\xf0,0\x91\x9f\xbfO\xf9\xff\xaf\xe2U\xc0<w\xb0\\027\xb3c\xab\\)Q\xb1\xba \x9cO\xb4.\xb4\xf9 xBtu\x83\xe5\\f!")
	if got := ExtractPDFSubject(pdfWithTitle(binary)); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
	if Readable(decodeLatin1(binary)) {
		t.Errorf("readable(%q) = true", decodeLatin1(binary))
	}
}

func TestWhatReadsAsATitleAndWhatDoesNot(t *testing.T) {
	noPDF(t)
	for _, good := range []string{"CHARBONE - Closing 2nd Drawdown", "D\u00e9claration de placement avec dispense 45-106F1",
		"Q3 2026 Interim Financial Statements", "Form 45-106F1 Report of Exempt Distribution"} {
		if !Readable(good) {
			t.Errorf("%s", good)
		}
	}
	for _, bad := range []string{"2026-09-04", "\u00b1\u00ba\u00b4\u00ab\u00b9\u00b2", "", "   ", "\x0c\x12 Report", "45-106"} {
		if Readable(bad) {
			t.Errorf("%q", bad)
		}
	}
}

func TestAnUnreadableTitleFallsBackToTheModel(t *testing.T) {
	t.Setenv("BAGHOLDER_NO_PDF", "")
	e := &Enricher{Model: modelAnswering(t, func(prompt string) string {
		if strings.Contains(prompt, "Title:") {
			return "Report of exempt distribution in Canada"
		}
		return "It reports a distribution."
	})}
	data := realPDF([]byte("\x8a\xf0,0\x91\x9f\xbfO\xf9\xff\xaf\xe2U\xc0<w\xb0"), "A report of exempt distribution.")
	out := e.EnrichDocument("sedar", data, "application/pdf")
	if out.Subject != "Report of exempt distribution in Canada" {
		t.Errorf("got %q, want %q", out.Subject, "Report of exempt distribution in Canada")
	}
}

func TestNonPDFBytesHaveNoPDFSubject(t *testing.T) {
	noPDF(t)
	if got := ExtractPDFSubject([]byte("<html>...</html>")); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestHTMLTextDropsScriptsAndTags(t *testing.T) {
	noPDF(t)
	html := []byte("<html><head><style>.x{}</style></head><body><h1>Results</h1><script>x()</script><p>Net income up 20%</p></body></html>")
	if got := HTMLText(html); got != "Results Net income up 20%" {
		t.Errorf("got %q, want %q", got, "Results Net income up 20%")
	}
}

func TestPDFTextIsEmptyWhenNoEngine(t *testing.T) {
	noPDF(t)
	if got := PDFTextClean([]byte("%PDF-1.7 ...")); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestDocumentTextRoutesByType(t *testing.T) {
	noPDF(t)
	if got := DocumentText([]byte("<p>hello there</p>"), "text/html"); got != "hello there" {
		t.Errorf("got %q, want %q", got, "hello there")
	}
}

func TestNoModelMeansNoSummary(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelDown(t)}
	if got := e.Summarize("Some filing text"); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestSummaryOfEmptyTextIsEmpty(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelDown(t)}
	if got := e.Summarize(""); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestSummaryIsKeptToOneSentence(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelSaying(t, "It announces a private placement. Extra sentence.")}
	if got := e.Summarize("text"); got != "It announces a private placement." {
		t.Errorf("got %q, want %q", got, "It announces a private placement.")
	}
}

func TestSummaryStripsChatTemplateTokens(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelSaying(t, "It announces a private placement.<|eot_id|>")}
	if got := e.Summarize("text"); got != "It announces a private placement." {
		t.Errorf("got %q, want %q", got, "It announces a private placement.")
	}
}

func TestTitleStripsAChattyPreambleAndMarkdown(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelSaying(t, "Sure, here is the title: **Closing of $1.5M Drawdown**")}
	if got := e.TitleFromModel("some filing text"); got != "Closing of $1.5M Drawdown" {
		t.Errorf("got %q, want %q", got, "Closing of $1.5M Drawdown")
	}
}

func TestTitleRejectsABareFormCodeOrEcho(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelSaying(t, "Schedule 13G")}
	if got := e.TitleFromModel("text"); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
	e = &Enricher{Model: modelSaying(t, "Here is a title for the filing")}
	if got := e.TitleFromModel("text"); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestNoModelMeansNoTitle(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelDown(t)}
	if got := e.TitleFromModel("text"); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestEnrichDocumentTitlesFromTheModelWhenThereIsNoPDFSubject(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelAnswering(t, func(prompt string) string {
		if strings.Contains(prompt, "Title:") {
			return "Q2 2026 MD&A and interim financial statements"
		}
		return "It reports Q2 2026 results."
	})}
	info := e.EnrichDocument("SEC", []byte("<html><body>Management discussion...</body></html>"), "text/html")
	if info.Subject != "Q2 2026 MD&A and interim financial statements" {
		t.Errorf("got %q, want %q", info.Subject, "Q2 2026 MD&A and interim financial statements")
	}
	if info.Summary != "It reports Q2 2026 results." {
		t.Errorf("got %q, want %q", info.Summary, "It reports Q2 2026 results.")
	}
}

func TestEnrichDocumentGivesSubjectWithoutAModel(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelDown(t)}
	info := e.EnrichDocument("SEDAR+", pdfWithTitle([]byte("Microsoft Word - Acme Announces Buyback EN")), "application/pdf")
	if info.Subject != "Acme Announces Buyback" {
		t.Errorf("got %q, want %q", info.Subject, "Acme Announces Buyback")
	}
	if info.Summary != "" {
		t.Errorf("got %q, want %q", info.Summary, "")
	}
}

func TestACompanySuffixDoesNotEndTheSentence(t *testing.T) {
	noPDF(t)
	for _, opening := range []string{"Quantum eMotion Corp.", "Aegis Critical Energy Defence Corp.", "Shopify Inc.",
		"High Tide Ltd.", "Brookfield Co.", "Barrick PLC"} {
		line := opening + " announces a commercial order for seven units in the United States."
		if got := FirstSentence(line); got != line {
			t.Errorf("%s: got %q, want %q", opening, got, line)
		}
	}
}

func TestAnInitialOrAnAbbreviationDoesNotEndItEither(t *testing.T) {
	noPDF(t)
	for _, line := range []string{"U.S. regulators approved the base shelf prospectus.",
		"Dr. Chen was appointed chief scientist of the company.",
		"No. 4 of the schedule lists the securities offered.",
		"J. Smith resigned from the board of directors."} {
		if got := FirstSentence(line); got != line {
			t.Errorf("%s: got %q, want %q", line, got, line)
		}
	}
}

func TestARealSecondSentenceIsDropped(t *testing.T) {
	noPDF(t)
	if got := FirstSentence("The company files Q1 statements. It also names a director."); got != "The company files Q1 statements." {
		t.Errorf("got %q, want %q", got, "The company files Q1 statements.")
	}
	if got := FirstSentence("Is the prospectus final? The company says yes."); got != "Is the prospectus final?" {
		t.Errorf("got %q, want %q", got, "Is the prospectus final?")
	}
}

func TestAStopFollowedByMoreOfTheSameSentenceIsNotAnEnding(t *testing.T) {
	noPDF(t)
	line := "The filing lists exhibits 1.2 and 3. and describes the securities offered."
	if got := FirstSentence(line); got != line {
		t.Errorf("got %q, want %q", got, line)
	}
}

func TestAnAnswerWithNoStopAtAllSurvivesWhole(t *testing.T) {
	noPDF(t)
	if got := FirstSentence("Quantum eMotion Corp files its interim statements"); got != "Quantum eMotion Corp files its interim statements" {
		t.Errorf("got %q, want %q", got, "Quantum eMotion Corp files its interim statements")
	}
	if got := FirstSentence(""); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}

func TestTheSummaryKeepsTheWholeSentenceRatherThanTheNameAlone(t *testing.T) {
	noPDF(t)
	said := "Quantum eMotion Corp. announces its participation as a sponsor of the AI for Good Global Summit."
	e := &Enricher{Model: modelSaying(t, said)}
	if got := e.Summarize("the filing's text"); got != said {
		t.Errorf("got %q, want %q", got, said)
	}
}

func TestABareNameIsStillNoSummary(t *testing.T) {
	noPDF(t)
	e := &Enricher{Model: modelSaying(t, "Quantum eMotion Corp.")}
	if got := e.Summarize("the filing's text"); got != "" {
		t.Errorf("got %q, want %q", got, "")
	}
}
