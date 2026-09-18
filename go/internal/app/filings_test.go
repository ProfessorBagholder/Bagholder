package app

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"sort"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/disclosures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const secSource = "SEC"

func filingItem(source string, i int, profile string) store.FilingItem {
	tag := strings.ToLower(strings.ReplaceAll(strings.Split(source, "+")[0], " ", ""))
	it := store.FilingItem{
		ID:        fmt.Sprintf("%s:%d", tag, i),
		Source:    source,
		Category:  "Financials",
		Date:      fmt.Sprintf("2026-08-%02d", 10+i),
		DateText:  fmt.Sprintf("2026-08-%02d", 10+i),
		Type:      "Interim MD&A",
		Size:      "292 KB",
		URL:       fmt.Sprintf("https://www.sedarplus.ca/x?drmKey=%d", i),
		ProfileNo: profile,
	}
	if source == secSource {
		it.Type, it.Title, it.Size = "10-Q", "Quarterly report", ""
		it.URL = fmt.Sprintf("https://www.sec.gov/x/%d", i)
	}
	return it
}

func asItem(it store.FilingItem) disclosures.Item {
	return disclosures.Item{ID: it.ID, Source: it.Source, Category: it.Category, Date: it.Date, DateText: it.DateText, Type: it.Type, Title: it.Title, Size: it.Size, URL: it.URL, ProfileNo: it.ProfileNo}
}

type fakeProvider struct {
	source      string
	available   bool
	covers      bool
	items       []disclosures.Item
	err         error
	filer       bool
	content     []byte
	contentType string
	contentErr  error
	enrichment  *disclosures.Enrichment

	mu    sync.Mutex
	reads int
}

func (f *fakeProvider) Source() string                                { return f.source }
func (f *fakeProvider) Available() bool                               { return f.available }
func (f *fakeProvider) Covers(symbol, exchange, currency string) bool { return f.covers }
func (f *fakeProvider) Fetch(symbol, name, exchange, currency string, limit int, profileNo string) ([]disclosures.Item, error) {
	if f.err != nil {
		return nil, f.err
	}
	return f.items, nil
}
func (f *fakeProvider) Document(row disclosures.Row) ([]byte, string, error) {
	return f.content, f.contentType, f.contentErr
}
func (f *fakeProvider) HasFiler(symbol, name, exchange, currency string) (bool, bool) {
	return f.filer, f.filer
}
func (f *fakeProvider) Enrichment(row disclosures.Row) *disclosures.Enrichment { return f.enrichment }
func (f *fakeProvider) Categorize(row disclosures.Row) string                  { return row.Category }
func (f *fakeProvider) Content(row disclosures.Row) ([]byte, string, error) {
	f.mu.Lock()
	f.reads++
	f.mu.Unlock()
	return f.content, f.contentType, f.contentErr
}
func (f *fakeProvider) calls() int {
	f.mu.Lock()
	defer f.mu.Unlock()
	return f.reads
}

func sources(out map[string]any) map[string]map[string]bool {
	return out["sources"].(map[string]map[string]bool)
}

func filingsOf(t *testing.T, out map[string]any) []store.Filing {
	t.Helper()
	rows, ok := out["filings"].([]store.Filing)
	if !ok {
		t.Fatalf("filings: %#v", out["filings"])
	}
	return rows
}

func TestRowsFromTwoSourcesMergeNewestFirst(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("SHOP", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, ""), filingItem(disclosures.SedarSource, 3, "")}, "")
	a.st.ReplaceFilings("SHOP", secSource, []store.FilingItem{filingItem(secSource, 2, ""), filingItem(secSource, 4, "")}, "")
	rows := a.st.Filings("SHOP")
	if len(rows) != 4 {
		t.Fatalf("rows = %d, want 4", len(rows))
	}
	dates := []string{}
	for _, r := range rows {
		dates = append(dates, r.Date)
	}
	want := append([]string{}, dates...)
	sort.Sort(sort.Reverse(sort.StringSlice(want)))
	if !equalStrings(dates, want) {
		t.Errorf("dates = %v, want newest first %v", dates, want)
	}
	seen := map[string]bool{}
	for _, r := range rows {
		seen[r.Source] = true
	}
	if !seen[disclosures.SedarSource] || !seen[secSource] || len(seen) != 2 {
		t.Errorf("sources = %v", seen)
	}
}

func TestReplacingOneSourceLeavesTheOther(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("SHOP", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, ""), filingItem(disclosures.SedarSource, 2, "")}, "")
	a.st.ReplaceFilings("SHOP", secSource, []store.FilingItem{filingItem(secSource, 1, "")}, "")
	a.st.ReplaceFilings("SHOP", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 9, "")}, "")
	rows := a.st.Filings("SHOP")
	got := []string{}
	sedar, sec := 0, 0
	for _, r := range rows {
		got = append(got, r.Source)
		if r.Source == disclosures.SedarSource {
			sedar++
		}
		if r.Source == secSource {
			sec++
		}
	}
	sort.Strings(got)
	if !equalStrings(got, []string{secSource, disclosures.SedarSource}) {
		t.Errorf("sources = %v", got)
	}
	if sedar != 1 {
		t.Errorf("SEDAR+ rows = %d: SEDAR+ replaced, not appended", sedar)
	}
	if sec != 1 {
		t.Errorf("SEC rows = %d: SEC untouched", sec)
	}
}

func TestASingleRowIsFetchableByIDForDownload(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("SHOP", secSource, []store.FilingItem{filingItem(secSource, 7, "")}, "")
	row := a.st.Filing("SHOP", "sec:7")
	if row == nil {
		t.Fatal("sec:7 is not stored")
	}
	if row.Source != secSource {
		t.Errorf("source = %q", row.Source)
	}
	if !strings.HasPrefix(row.URL, "https://www.sec.gov/") {
		t.Errorf("url = %q", row.URL)
	}
	if got := a.st.Filing("SHOP", "sec:999"); got != nil {
		t.Errorf("sec:999 = %+v, want none", got)
	}
}

func TestSymbolsDoNotBleedAndTheProfileIsRemembered(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("SHOP", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, "")}, "")
	a.st.ReplaceFilings("ATD", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, ""), filingItem(disclosures.SedarSource, 2, "")}, "")
	a.st.MarkFilingsFetched("ATD", "000012345", "")
	if got := len(a.st.Filings("SHOP")); got != 1 {
		t.Errorf("SHOP rows = %d, want 1", got)
	}
	if got := len(a.st.Filings("ATD")); got != 2 {
		t.Errorf("ATD rows = %d, want 2", got)
	}
	if got := a.st.SedarProfile("ATD"); got != "000012345" {
		t.Errorf("profile = %q", got)
	}
}

func TestForgetClearsRowsAndStamps(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("SHOP", secSource, []store.FilingItem{filingItem(secSource, 1, "")}, "")
	a.st.MarkFilingsFetched("SHOP", "000037100", "")
	a.st.ForgetFilings("SHOP")
	if got := a.st.Filings("SHOP"); len(got) != 0 {
		t.Errorf("rows = %v, want none", got)
	}
	if got := a.st.FilingsFetchedAt("SHOP"); got != "" {
		t.Errorf("fetchedAt = %q", got)
	}
	if got := a.st.SedarProfile("SHOP"); got != "" {
		t.Errorf("profile = %q", got)
	}
}

func TestDataSummaryCountsFilings(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("SHOP", secSource, []store.FilingItem{filingItem(secSource, 1, ""), filingItem(secSource, 2, "")}, "")
	a.st.ReplaceFilings("SHOP", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, "")}, "")
	if got := a.st.DataSummary()["filings"]; got != 3 {
		t.Errorf("filings = %v, want 3", got)
	}
}

func payloadApp(t *testing.T, sedar, sec *fakeProvider) *App {
	t.Helper()
	a := newTestApp(t)
	a.pipeline = &disclosures.Pipeline{Providers: []disclosures.Provider{sedar, sec}, Sedar: sedar}
	return a
}

func TestStaleUntilAFetchThenFreshWithinADay(t *testing.T) {
	a := newTestApp(t)
	if !a.filingsStale("SHOP", time.Now().UTC(), FilingsStaleHours) {
		t.Error("a symbol never fetched is fresh")
	}
	a.st.MarkFilingsFetched("SHOP", "", "")
	if a.filingsStale("SHOP", time.Now().UTC(), FilingsStaleHours) {
		t.Error("a symbol just fetched is stale")
	}
}

func TestADayOldStampIsStale(t *testing.T) {
	a := newTestApp(t)
	old := time.Now().UTC().Add(-25 * time.Hour).Format("2006-01-02T15:04:05Z")
	a.st.MarkFilingsFetched("SHOP", "", old)
	if !a.filingsStale("SHOP", time.Now().UTC(), FilingsStaleHours) {
		t.Error("a stamp 25 hours old is fresh")
	}
}

func TestRefreshMergesSourcesAndReportsStatus(t *testing.T) {
	sedar := &fakeProvider{source: disclosures.SedarSource, available: true, covers: true, items: []disclosures.Item{asItem(filingItem(disclosures.SedarSource, 1, "000037100"))}}
	sec := &fakeProvider{source: secSource, available: true, covers: true, items: []disclosures.Item{asItem(filingItem(secSource, 2, ""))}}
	a := payloadApp(t, sedar, sec)
	out := a.filingsPayload("SHOP", true, "", nil, nil)
	if out["ok"] != true {
		t.Errorf("ok = %v", out["ok"])
	}
	if out["available"] != true {
		t.Errorf("available = %v", out["available"])
	}
	if out["refreshed"] != true {
		t.Errorf("refreshed = %v", out["refreshed"])
	}
	if got := filingsOf(t, out); len(got) != 2 {
		t.Errorf("filings = %d, want 2", len(got))
	}
	if out["profileNo"] != "000037100" {
		t.Errorf("profileNo = %v: the SEDAR+ profile is remembered from the items", out["profileNo"])
	}
	names := []string{}
	for src := range sources(out) {
		names = append(names, src)
	}
	sort.Strings(names)
	if !equalStrings(names, []string{secSource, disclosures.SedarSource}) {
		t.Errorf("sources = %v", names)
	}
	found := false
	for _, c := range out["categories"].([]string) {
		if c == "Financials" {
			found = true
		}
	}
	if !found {
		t.Errorf("categories = %v, want Financials among them", out["categories"])
	}
}

func TestOnlyOneSourceMatches(t *testing.T) {
	sedar := &fakeProvider{source: disclosures.SedarSource, available: true, covers: true}
	sec := &fakeProvider{source: secSource, available: true, covers: true, items: []disclosures.Item{asItem(filingItem(secSource, 1, ""))}}
	a := payloadApp(t, sedar, sec)
	out := a.filingsPayload("NVDA", true, "", nil, nil)
	got := []string{}
	for _, r := range filingsOf(t, out) {
		got = append(got, r.Source)
	}
	if !equalStrings(got, []string{secSource}) {
		t.Errorf("filings sources = %v, want [SEC]", got)
	}
	if !sources(out)[secSource]["matched"] {
		t.Error("SEC did not match")
	}
	if sources(out)[disclosures.SedarSource]["matched"] {
		t.Error("SEDAR+ matched")
	}
}

func TestAllSourcesUnreachableIsReported(t *testing.T) {
	sedar := &fakeProvider{source: disclosures.SedarSource, available: false, covers: true}
	sec := &fakeProvider{source: secSource, available: false, covers: true}
	a := payloadApp(t, sedar, sec)
	out := a.filingsPayload("SHOP", true, "", nil, nil)
	if out["ok"] != true {
		t.Errorf("ok = %v: the endpoint still answers cleanly", out["ok"])
	}
	if out["sourceUnavailable"] != true {
		t.Errorf("sourceUnavailable = %v", out["sourceUnavailable"])
	}
	if got := filingsOf(t, out); len(got) != 0 {
		t.Errorf("filings = %v, want none", got)
	}
}

func TestAnEmptySymbolIsRejected(t *testing.T) {
	a := newTestApp(t)
	if out := a.filingsPayload("", false, "", nil, nil); out["ok"] != false {
		t.Errorf("ok = %v, want false", out["ok"])
	}
}

type modelStub struct {
	srv     *httptest.Server
	mu      sync.Mutex
	up      bool
	probes  int
	title   string
	summary string
}

func newModelStub(t *testing.T) *modelStub {
	t.Helper()
	m := &modelStub{up: true}
	m.srv = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		m.mu.Lock()
		up, title, summary := m.up, m.title, m.summary
		if strings.HasSuffix(r.URL.Path, "/v1/models") {
			m.probes++
		}
		m.mu.Unlock()
		if !up {
			w.WriteHeader(500)
			return
		}
		if strings.HasSuffix(r.URL.Path, "/v1/models") {
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"data":[{"id":"local"}]}`))
			return
		}
		body, _ := io.ReadAll(r.Body)
		answer := title
		if strings.Contains(string(body), "SUMMARY (one sentence)") {
			answer = summary
		}
		raw, _ := json.Marshal(map[string]any{"choices": []map[string]any{{"message": map[string]string{"content": answer}}}})
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write(raw)
	}))
	t.Cleanup(m.srv.Close)
	return m
}

func (m *modelStub) set(up bool, title, summary string) {
	m.mu.Lock()
	m.up, m.title, m.summary = up, title, summary
	m.mu.Unlock()
}

func (m *modelStub) probeCount() int {
	m.mu.Lock()
	defer m.mu.Unlock()
	return m.probes
}

const modelSummary = "The company filed its quarterly financial statements."
const modelTitle = "Quarterly financial statements"

func htmlDoc(body string) []byte {
	return []byte("<html><body><p>" + body + "</p></body></html>")
}

func pdfDoc(title string) []byte {
	return []byte("%PDF-1.4\n/Title (" + title + ")\n%%EOF\n")
}

const docBody = "Quantum eMotion Corp reported its results for the quarter ended 30 June 2026 and filed the related statements."

const formBody = "Form 45-106F1 Report of Exempt Distribution Total dollar amount of securities distributed $1,500,000.0000"

const blankFormBody = "(YYYY-MM-DD) refer to Part 3 select one complete Schedule 1 if applicable check the box"

type enrichFixture struct {
	t    *testing.T
	home string
	fp   *fakeProvider
	stub *modelStub
}

func newEnrichFixture(t *testing.T) *enrichFixture {
	t.Helper()
	stub := newModelStub(t)
	t.Setenv("BAGHOLDER_LLM_URL", stub.srv.URL)
	t.Setenv("BAGHOLDER_OLLAMA_URL", "http://127.0.0.1:1")
	t.Setenv("BAGHOLDER_LLAMAFILE_URL", "http://127.0.0.1:1/x")
	fx := &enrichFixture{t: t, home: t.TempDir(), fp: &fakeProvider{source: disclosures.SedarSource, available: true, covers: true, contentType: "text/html"}, stub: stub}
	a := fx.app()
	defer a.st.Close()
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, "")}, "")
	return fx
}

func (fx *enrichFixture) app() *App {
	fx.t.Helper()
	a, err := New(Config{Home: fx.home, OrdersLive: false, BindHost: "127.0.0.1"})
	if err != nil {
		fx.t.Fatal(err)
	}
	a.pipeline = &disclosures.Pipeline{Providers: []disclosures.Provider{fx.fp}, Sedar: fx.fp}
	return a
}

func (fx *enrichFixture) enrich(modelUp bool, doc []byte, title, summary string) (map[string]any, int) {
	fx.t.Helper()
	fx.stub.set(modelUp, title, summary)
	fx.fp.content = doc
	fx.fp.contentType = "text/html"
	if len(doc) >= 5 && string(doc[:5]) == "%PDF-" {
		fx.fp.contentType = "application/pdf"
	}
	before := fx.fp.calls()
	a := fx.app()
	defer a.st.Close()
	out := a.filingsEnrich("QNC", "sedar:1")
	return out, fx.fp.calls() - before
}

func (fx *enrichFixture) stored() (string, string, int) {
	fx.t.Helper()
	a := fx.app()
	defer a.st.Close()
	row := a.st.Filing("QNC", "sedar:1")
	if row == nil {
		fx.t.Fatal("sedar:1 is gone")
	}
	return row.Subject, row.Summary, row.EnrichVersion
}

func TestARowWithBothHalvesIsNotReadAgain(t *testing.T) {
	fx := newEnrichFixture(t)
	fx.enrich(true, htmlDoc(docBody), modelTitle, modelSummary)
	if s, sum, _ := fx.stored(); s != modelTitle || sum != modelSummary {
		t.Fatalf("stored = (%q, %q)", s, sum)
	}
	out, reads := fx.enrich(true, htmlDoc(docBody), "Another title entirely", "Another sentence about the filing.")
	if reads != 0 {
		t.Errorf("reads = %d: the document is not fetched a second time", reads)
	}
	if out["subject"] != modelTitle {
		t.Errorf("subject = %v", out["subject"])
	}
}

func TestADocumentReadForGoodIsNeverFetchedAgainEvenWithNothingToShow(t *testing.T) {
	fx := newEnrichFixture(t)
	out, _ := fx.enrich(true, htmlDoc(blankFormBody), modelTitle, modelSummary)
	if out["subject"] != "" || out["summary"] != "" {
		t.Errorf("(subject, summary) = (%v, %v), want empty", out["subject"], out["summary"])
	}
	if _, _, v := fx.stored(); v != EnrichVersion {
		t.Errorf("enrichVersion = %d, want %d: stamped, so the row is done", v, EnrichVersion)
	}
	for i := 0; i < 3; i++ {
		_, reads := fx.enrich(true, htmlDoc(blankFormBody), modelTitle, modelSummary)
		if reads != 0 {
			t.Errorf("pass %d: reads = %d: the document is not fetched again", i, reads)
		}
	}
}

func TestADocumentReadForGoodWhileNoModelWasUpIsStillDone(t *testing.T) {
	fx := newEnrichFixture(t)
	fx.enrich(false, htmlDoc(formBody), "", "")
	if s, sum, _ := fx.stored(); s != "Exempt distribution of $1,500,000" || sum != "$1,500,000 distributed." {
		t.Fatalf("stored = (%q, %q)", s, sum)
	}
	_, reads := fx.enrich(true, htmlDoc(docBody), "Another title entirely", "Another sentence about the filing.")
	if reads != 0 {
		t.Errorf("reads = %d: a form needs no model, so a model arriving later changes nothing", reads)
	}
}

func TestARowHoldingOnlyATitleIsReadAgainForItsSummary(t *testing.T) {
	fx := newEnrichFixture(t)
	fx.enrich(true, htmlDoc(docBody), modelTitle, "")
	if s, sum, _ := fx.stored(); s != modelTitle || sum != "" {
		t.Fatalf("stored = (%q, %q)", s, sum)
	}
	out, reads := fx.enrich(true, htmlDoc(docBody), modelTitle, modelSummary)
	if reads != 1 {
		t.Errorf("reads = %d, want 1", reads)
	}
	if out["summary"] != modelSummary {
		t.Errorf("summary = %v", out["summary"])
	}
}

func TestARowHoldingOnlyASummaryIsReadAgainForItsTitle(t *testing.T) {
	fx := newEnrichFixture(t)
	fx.enrich(true, htmlDoc(docBody), "", modelSummary)
	if s, sum, _ := fx.stored(); s != "" || sum != modelSummary {
		t.Fatalf("stored = (%q, %q)", s, sum)
	}
	out, reads := fx.enrich(true, htmlDoc(docBody), modelTitle, modelSummary)
	if reads != 1 {
		t.Errorf("reads = %d, want 1", reads)
	}
	if out["subject"] != modelTitle {
		t.Errorf("subject = %v", out["subject"])
	}
}

func TestReadingAgainFillsWhatIsMissingAndEmptiesNothing(t *testing.T) {
	fx := newEnrichFixture(t)
	fx.enrich(true, htmlDoc(docBody), modelTitle, "")
	out, reads := fx.enrich(true, htmlDoc(docBody), "", "")
	if reads != 1 {
		t.Errorf("reads = %d, want 1", reads)
	}
	if out["subject"] != modelTitle {
		t.Errorf("subject = %v: what was already there survives", out["subject"])
	}
	if s, _, _ := fx.stored(); s != modelTitle {
		t.Errorf("stored subject = %q", s)
	}
}

func TestTheFirstReadUnderTheCurrentLogicStillClearsAnOlderJunkTitle(t *testing.T) {
	fx := newEnrichFixture(t)
	func() {
		a := fx.app()
		defer a.st.Close()
		subject, summary, version := "00012345.pdf", "", 1
		a.st.SetFilingEnrichment("QNC", "sedar:1", &subject, &summary, &version, nil)
	}()
	out, reads := fx.enrich(true, htmlDoc(docBody), "", modelSummary)
	if reads != 1 {
		t.Errorf("reads = %d, want 1", reads)
	}
	if out["subject"] != "" {
		t.Errorf("subject = %v: the stale title goes rather than sticking", out["subject"])
	}
	if out["summary"] != modelSummary {
		t.Errorf("summary = %v", out["summary"])
	}
}

func TestWithNoModelUpARowAlreadyReadIsNotFetchedAgain(t *testing.T) {
	fx := newEnrichFixture(t)
	fx.enrich(true, htmlDoc(docBody), modelTitle, "")
	out, reads := fx.enrich(false, htmlDoc(docBody), "", "never asked")
	if reads != 0 {
		t.Errorf("reads = %d: no summary is coming, nothing to gain by reading", reads)
	}
	if out["summary"] != "" {
		t.Errorf("summary = %v", out["summary"])
	}
}

func TestARowNeverReadIsReadEvenWithNoModel(t *testing.T) {
	fx := newEnrichFixture(t)
	out, reads := fx.enrich(false, pdfDoc("A title"), "", "")
	if reads != 1 {
		t.Errorf("reads = %d, want 1: the document's own title is still worth having", reads)
	}
	if out["subject"] != "A title" {
		t.Errorf("subject = %v", out["subject"])
	}
	if _, _, v := fx.stored(); v != 0 {
		t.Errorf("enrichVersion = %d, want 0: it is read again once a model is up", v)
	}
}

func TestTheStatusSaysWhetherASummaryCouldBeMadeNow(t *testing.T) {
	stub := newModelStub(t)
	t.Setenv("BAGHOLDER_LLM_URL", stub.srv.URL)
	t.Setenv("BAGHOLDER_OLLAMA_URL", "http://127.0.0.1:1")
	t.Setenv("BAGHOLDER_LLAMAFILE_URL", "http://127.0.0.1:1/x")
	stub.set(true, modelTitle, modelSummary)
	a := newTestApp(t)
	if !a.enricher.SummaryAvailable() {
		t.Fatal("the stub model is not seen as up")
	}
	if a.statusPayload()["summaryReady"] != true {
		t.Error("summaryReady = false with a model ready")
	}
	stub.set(false, "", "")
	down := newTestApp(t)
	if down.statusPayload()["summaryReady"] != false {
		t.Error("summaryReady = true with no model up")
	}
}

func TestAskingTheStatusNeverStartsAModel(t *testing.T) {
	stub := newModelStub(t)
	stub.set(false, "", "")
	t.Setenv("BAGHOLDER_LLM_URL", stub.srv.URL)
	t.Setenv("BAGHOLDER_OLLAMA_URL", "http://127.0.0.1:1")
	t.Setenv("BAGHOLDER_LLAMAFILE_URL", "http://127.0.0.1:1/x")
	a := newTestApp(t)
	before := stub.probeCount()
	if a.statusPayload()["summaryReady"] != false {
		t.Error("summaryReady = true with no model up")
	}
	if got := stub.probeCount() - before; got != 0 {
		t.Errorf("the status poll probed the model %d times: a status poll started a model", got)
	}
}

func TestAModelThatIsStartingIsWaitedForRatherThanTheReadWasted(t *testing.T) {
	fx := newEnrichFixture(t)
	fx.stub.set(false, modelTitle, modelSummary)
	fx.fp.content = htmlDoc(docBody)
	fx.fp.contentType = "text/html"
	a := fx.app()
	defer a.st.Close()
	go func() {
		time.Sleep(200 * time.Millisecond)
		fx.stub.set(true, modelTitle, modelSummary)
	}()
	out := a.filingsEnrich("QNC", "sedar:1")
	if out["summary"] != modelSummary {
		t.Errorf("summary = %v, want %q", out["summary"], modelSummary)
	}
	s, sum, v := fx.stored()
	if s != modelTitle || sum != modelSummary || v != EnrichVersion {
		t.Errorf("stored = (%q, %q, %d)", s, sum, v)
	}
}

func TestAModelThatNeverComesUpLeavesTheRowToBeReadAgain(t *testing.T) {
	fx := newEnrichFixture(t)
	out, _ := fx.enrich(false, pdfDoc("A title"), "", "")
	if out["subject"] != "A title" {
		t.Errorf("subject = %v", out["subject"])
	}
	if _, _, v := fx.stored(); v != 0 {
		t.Errorf("enrichVersion = %d, want 0: read again once a model is up", v)
	}
}

func TestARowTheSourceStillListsKeepsItsSubjectAndSummary(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, ""), filingItem(disclosures.SedarSource, 2, "")}, "")
	subject, summary, version := "A title", "A sentence.", 9
	a.st.SetFilingEnrichment("QNC", "sedar:1", &subject, &summary, &version, nil)
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, ""), filingItem(disclosures.SedarSource, 2, ""), filingItem(disclosures.SedarSource, 3, "")}, "")
	row := a.st.Filing("QNC", "sedar:1")
	if row == nil {
		t.Fatal("sedar:1 is gone")
	}
	if row.Subject != "A title" || row.Summary != "A sentence." || row.EnrichVersion != 9 {
		t.Errorf("(subject, summary, enrichVersion) = (%q, %q, %d)", row.Subject, row.Summary, row.EnrichVersion)
	}
}

func TestWhatTheSourceSaysAboutARowIsStillRefreshed(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, "")}, "")
	subject, summary, version := "A title", "A sentence.", 9
	a.st.SetFilingEnrichment("QNC", "sedar:1", &subject, &summary, &version, nil)
	moved := filingItem(disclosures.SedarSource, 1, "")
	moved.URL = "https://www.sedarplus.ca/x?drmKey=fresh"
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{moved}, "")
	row := a.st.Filing("QNC", "sedar:1")
	if row == nil {
		t.Fatal("sedar:1 is gone")
	}
	if row.URL != "https://www.sedarplus.ca/x?drmKey=fresh" {
		t.Errorf("url = %q", row.URL)
	}
	if row.Summary != "A sentence." {
		t.Errorf("summary = %q", row.Summary)
	}
}

func TestARowTheSourceNoLongerListsGoes(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, ""), filingItem(disclosures.SedarSource, 2, "")}, "")
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 2, "")}, "")
	if got := a.st.Filing("QNC", "sedar:1"); got != nil {
		t.Errorf("sedar:1 = %+v, want none", got)
	}
	if got := a.st.Filing("QNC", "sedar:2"); got == nil {
		t.Error("sedar:2 is gone")
	}
}

func TestAnotherSourcesRowsAreUntouched(t *testing.T) {
	a := newTestApp(t)
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, "")}, "")
	a.st.ReplaceFilings("QNC", secSource, []store.FilingItem{filingItem(secSource, 1, "")}, "")
	subject, summary, version := "From EDGAR", "A sentence.", 9
	a.st.SetFilingEnrichment("QNC", "sec:1", &subject, &summary, &version, nil)
	a.st.ReplaceFilings("QNC", disclosures.SedarSource, []store.FilingItem{filingItem(disclosures.SedarSource, 1, "")}, "")
	row := a.st.Filing("QNC", "sec:1")
	if row == nil {
		t.Fatal("sec:1 is gone")
	}
	if row.Subject != "From EDGAR" {
		t.Errorf("subject = %q", row.Subject)
	}
}

func TestAReadNoSourceAnsweredIsNotStamped(t *testing.T) {
	sedar := &fakeProvider{source: disclosures.SedarSource, available: false, covers: true}
	sec := &fakeProvider{source: secSource, available: false, covers: true}
	a := payloadApp(t, sedar, sec)
	out := a.filingsPayload("CH", true, "", nil, nil)
	if out["sourceUnavailable"] != true {
		t.Fatalf("sourceUnavailable = %v", out["sourceUnavailable"])
	}
	if got := a.st.FilingsFetchedAt("CH"); got != "" {
		t.Errorf("fetched stamp = %q after a read no source answered, want none", got)
	}
	if !a.filingsStale("CH", time.Now().UTC(), FilingsStaleHours) {
		t.Error("a symbol whose read no source answered is fresh")
	}
	sedar.available = true
	out = a.filingsPayload("CH", false, "", nil, nil)
	if out["sourceUnavailable"] == true {
		t.Error("the next ask did not read again")
	}
	if a.st.FilingsFetchedAt("CH") == "" {
		t.Error("a read a source answered is not stamped")
	}
}
