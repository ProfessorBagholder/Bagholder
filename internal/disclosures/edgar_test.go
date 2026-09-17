package disclosures

import (
	"bytes"
	"errors"
	"io"
	"net/http"
	"strings"
	"sync"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const submissionsJSON = `{"name":"NVIDIA CORP","filings":{"recent":{` +
	`"form":["10-Q","8-K","4","SCHEDULE 13D/A","424B5","DEF 14A","NT 10-K"],` +
	`"filingDate":["2026-08-05","2026-08-01","2026-07-30","2026-07-20","2026-07-10","2026-06-15","2026-06-01"],` +
	`"primaryDocument":["nvda-10q.htm","nvda-8k.htm","form4.xml","sc13da.htm","424b5.htm","proxy.htm",""],` +
	`"accessionNumber":["0001-26-01","0001-26-02","0001-26-03","0001-26-04","0001-26-05","0001-26-06","0001-26-07"],` +
	`"primaryDocDescription":["","","","","","",""]}}}`

type stubReply struct {
	status int
	ct     string
	body   string
	err    error
}

type stubTransport struct {
	mu    sync.Mutex
	urls  []string
	reply func(u string) stubReply
}

func (s *stubTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	u := req.URL.String()
	s.mu.Lock()
	s.urls = append(s.urls, u)
	s.mu.Unlock()
	r := s.reply(u)
	if r.err != nil {
		return nil, r.err
	}
	h := http.Header{}
	if r.ct != "" {
		h.Set("Content-Type", r.ct)
	}
	return &http.Response{StatusCode: r.status, Status: http.StatusText(r.status), Header: h, Body: io.NopCloser(bytes.NewReader([]byte(r.body))), Request: req}, nil
}

func (s *stubTransport) documentURLs() []string {
	s.mu.Lock()
	defer s.mu.Unlock()
	var out []string
	for _, u := range s.urls {
		if !strings.HasSuffix(u, "/index.json") {
			out = append(out, u)
		}
	}
	return out
}

func newStubbedEdgar(t *testing.T, reply func(u string) stubReply) (*Edgar, *stubTransport) {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	m := market.NewClient(st)
	tr := &stubTransport{reply: reply}
	m.HTTP.Transport = tr
	return NewEdgar(m), tr
}

func newUnitEdgar(t *testing.T) *Edgar {
	t.Helper()
	e, _ := newStubbedEdgar(t, func(u string) stubReply {
		return stubReply{status: 200, ct: "application/json", body: submissionsJSON}
	})
	e.tickers = map[string]cikTitle{"NVDA": {1045810, "NVIDIA CORP"}, "SHOP": {1594805, "SHOPIFY INC."}}
	return e
}

func TestBareTickerStripsVenueAndDots(t *testing.T) {
	if got := EdgarBare("SHOP.TO"); got != "SHOP" {
		t.Errorf("EdgarBare(SHOP.TO) = %q", got)
	}
	if got := EdgarBare("BRK.B"); got != "BRK-B" {
		t.Errorf("EdgarBare(BRK.B) = %q", got)
	}
	if got := EdgarBare("nvda"); got != "NVDA" {
		t.Errorf("EdgarBare(nvda) = %q", got)
	}
}

func TestFormsMapToTheSharedCategories(t *testing.T) {
	cases := map[string]string{
		"10-Q": Financials, "40-F": Financials, "8-K": Events, "DEF 14A": Governance,
		"424B5": Offerings, "4": Insider, "SCHEDULE 13D/A": Insider, "NT 10-K": Other,
	}
	for form, want := range cases {
		if got := EdgarCategory(form); got != want {
			t.Errorf("EdgarCategory(%q) = %q, want %q", form, got, want)
		}
	}
}

func TestCoversAUSListingAndAKnownTicker(t *testing.T) {
	e := newUnitEdgar(t)
	if !e.Covers("NVDA", "NASDAQ", "USD") {
		t.Error("a US listing is covered")
	}
	if !e.Covers("SHOP", "TSX", "CAD") {
		t.Error("a cross-listed ticker SEC knows")
	}
	if e.Covers("QNC", "TSX-V", "CAD") {
		t.Error("a pure-Canadian ticker SEC does not know")
	}
}

func TestFetchNormalizesRowsWithSourceAndURL(t *testing.T) {
	e := newUnitEdgar(t)
	items, err := e.Fetch("NVDA", "NVIDIA Corporation", "NASDAQ", "USD", 200, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 7 {
		t.Fatalf("len(items) = %d", len(items))
	}
	first := items[0]
	if first.Source != "SEC" {
		t.Errorf("source = %q", first.Source)
	}
	if first.ID != "sec:0001-26-01" {
		t.Errorf("id = %q", first.ID)
	}
	if first.Type != "10-Q" {
		t.Errorf("type = %q", first.Type)
	}
	if first.Category != Financials {
		t.Errorf("category = %q", first.Category)
	}
	if !strings.HasPrefix(first.URL, "https://www.sec.gov/Archives/edgar/data/1045810/000126") {
		t.Errorf("url = %q", first.URL)
	}
}

func TestAMissingDocumentURLFallsBackToTheCompanyPage(t *testing.T) {
	e := newUnitEdgar(t)
	items, err := e.Fetch("NVDA", "", "NASDAQ", "USD", 200, "")
	if err != nil {
		t.Fatal(err)
	}
	var nt *Item
	for i := range items {
		if items[i].Type == "NT 10-K" {
			nt = &items[i]
			break
		}
	}
	if nt == nil {
		t.Fatal("no NT 10-K item")
	}
	if !strings.Contains(nt.URL, "browse-edgar") {
		t.Errorf("url = %q", nt.URL)
	}
}

func TestNameGuardRejectsACanadianTickerCollidingWithAUSFiler(t *testing.T) {
	e := newUnitEdgar(t)
	items, err := e.Fetch("NVDA", "Northvolt Canada Mining Corp.", "TSX-V", "CAD", 200, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 0 {
		t.Errorf("the SEC 'NVIDIA CORP' entity does not match the Canadian name: %v", items)
	}
}

func TestUnknownTickerReturnsEmpty(t *testing.T) {
	e := newUnitEdgar(t)
	items, err := e.Fetch("ZZZZ", "", "NASDAQ", "USD", 200, "")
	if err != nil {
		t.Fatal(err)
	}
	if len(items) != 0 {
		t.Errorf("items = %v", items)
	}
}

func TestMatchesIgnoreCorporateSuffixesAndCase(t *testing.T) {
	if !NamesMatch("Shopify Inc.", "SHOPIFY INC.") {
		t.Error("Shopify Inc. vs SHOPIFY INC.")
	}
	if !NamesMatch("NVIDIA Corporation", "NVIDIA CORP") {
		t.Error("NVIDIA Corporation vs NVIDIA CORP")
	}
}

func TestUnrelatedNamesDoNotMatch(t *testing.T) {
	if NamesMatch("Quantum eMotion Corp.", "QUALCOMM INC") {
		t.Error("Quantum eMotion Corp. vs QUALCOMM INC")
	}
	if NamesMatch("", "Anything") {
		t.Error("empty vs Anything")
	}
}

type fakeProv struct {
	source string
	items  []Item
	avail  bool
	covers bool
	err    error
}

func newFakeProv(source string, items []Item) *fakeProv {
	return &fakeProv{source: source, items: items, avail: true, covers: true}
}

func (p *fakeProv) Source() string                                { return p.source }
func (p *fakeProv) Available() bool                               { return p.avail }
func (p *fakeProv) Covers(symbol, exchange, currency string) bool { return p.covers }
func (p *fakeProv) Fetch(symbol, name, exchange, currency string, limit int, profileNo string) ([]Item, error) {
	if p.err != nil {
		return nil, p.err
	}
	return append([]Item{}, p.items...), nil
}
func (p *fakeProv) Document(row Row) ([]byte, string, error) {
	return []byte("%PDF-"), "application/pdf", nil
}
func (p *fakeProv) HasFiler(symbol, name, exchange, currency string) (bool, bool) {
	return false, false
}
func (p *fakeProv) Enrichment(row Row) *Enrichment          { return nil }
func (p *fakeProv) Categorize(row Row) string               { return "" }
func (p *fakeProv) Content(row Row) ([]byte, string, error) { return p.Document(row) }

func itemIDs(items []Item) []string {
	out := []string{}
	for _, it := range items {
		out = append(out, it.ID)
	}
	return out
}

func TestItemsFromBothProvidersMergeNewestFirst(t *testing.T) {
	a := newFakeProv("A", []Item{{ID: "a:1", Source: "A", Date: "2026-01-01"}})
	b := newFakeProv("B", []Item{{ID: "b:1", Source: "B", Date: "2026-05-01"}})
	p := &Pipeline{Providers: []Provider{a, b}}
	out := p.Fetch("X", "", "", "", 200, "")
	if got := itemIDs(out.Items); strings.Join(got, ",") != "b:1,a:1" {
		t.Errorf("items = %v", got)
	}
	if !out.Sources["A"].Matched {
		t.Error("A not matched")
	}
	if !out.Sources["B"].Matched {
		t.Error("B not matched")
	}
}

func TestAFailingSourceIsRecordedAndTheOtherStillReturns(t *testing.T) {
	a := newFakeProv("A", nil)
	a.err = Unavailable("down")
	b := newFakeProv("B", []Item{{ID: "b:1", Source: "B", Date: "2026-05-01"}})
	p := &Pipeline{Providers: []Provider{a, b}}
	out := p.Fetch("X", "", "", "", 200, "")
	if got := itemIDs(out.Items); strings.Join(got, ",") != "b:1" {
		t.Errorf("items = %v", got)
	}
	if out.Sources["A"].Available {
		t.Error("A reported available")
	}
	if !strings.Contains(out.Sources["A"].Error, "down") {
		t.Errorf("A error = %q", out.Sources["A"].Error)
	}
}

func TestASourceThatDoesNotCoverIsSkipped(t *testing.T) {
	a := newFakeProv("A", []Item{{ID: "a:1", Source: "A", Date: "2026-01-01"}})
	a.covers = false
	p := &Pipeline{Providers: []Provider{a}}
	out := p.Fetch("X", "", "", "", 200, "")
	if len(out.Items) != 0 {
		t.Errorf("items = %v", out.Items)
	}
	if out.Sources["A"].Matched {
		t.Error("A reported matched")
	}
}

func TestDocumentRoutesToTheRowsSource(t *testing.T) {
	a := newFakeProv("A", nil)
	p := &Pipeline{Providers: []Provider{a}}
	_, ct, err := p.Document(Row{Source: "A", ID: "a:1"})
	if err != nil {
		t.Fatal(err)
	}
	if ct != "application/pdf" {
		t.Errorf("content type = %q", ct)
	}
}

func TestItPicksTheLargestSubstantiveDocument(t *testing.T) {
	base := "https://www.sec.gov/Archives/edgar/data/2106613/000110465926097327"
	index := `{"directory":{"item":[` +
		`{"name":"0001104659-26-097327-index.html","size":3000},` +
		`{"name":"0001104659-26-097327.txt","size":106958},` +
		`{"name":"tm2623033d1_6k.htm","size":1230},` +
		`{"name":"tm2623033d1_ex99-1.htm","size":62339},` +
		`{"name":"tm2623033d1_ex99-3.htm","size":1223}]}}`
	e, tr := newStubbedEdgar(t, func(u string) stubReply {
		if strings.HasSuffix(u, "/index.json") {
			return stubReply{status: 200, ct: "application/json", body: index}
		}
		return stubReply{status: 200, ct: "text/html", body: "<html>MD&A</html>"}
	})
	if _, _, err := e.Content(Row{URL: base + "/tm2623033d1_6k.htm", Source: "SEC"}); err != nil {
		t.Fatal(err)
	}
	seen := tr.documentURLs()
	if len(seen) == 0 {
		t.Fatal("no document was read")
	}
	if !strings.HasSuffix(seen[0], "tm2623033d1_ex99-1.htm") {
		t.Errorf("document read = %q", seen[0])
	}
}

func TestItFallsBackToThePrimaryWhenTheIndexIsUnavailable(t *testing.T) {
	e, tr := newStubbedEdgar(t, func(u string) stubReply {
		if strings.HasSuffix(u, "/index.json") {
			return stubReply{err: errors.New("no index")}
		}
		return stubReply{status: 200, ct: "text/html", body: "x"}
	})
	e.Content(Row{URL: "https://www.sec.gov/Archives/edgar/data/1/2/primary.htm", Source: "SEC"})
	called := tr.documentURLs()
	if len(called) == 0 {
		t.Fatal("no document was read")
	}
	if !strings.HasSuffix(called[0], "primary.htm") {
		t.Errorf("document read = %q", called[0])
	}
}

func Test13GYieldsAStakeTitleAndSummary(t *testing.T) {
	xml := "<edgarSubmission><submissionType>SCHEDULE 13G/A</submissionType>" +
		"<issuerName>Quantum eMotion Corp.</issuerName>" +
		"<reportingPersonName>Capital Ventures International</reportingPersonName>" +
		"<reportingPersonName>Susquehanna Advisors Group, Inc.</reportingPersonName>" +
		"<classPercent>2.4</classPercent></edgarSubmission>"
	e, tr := newStubbedEdgar(t, func(u string) stubReply {
		return stubReply{status: 200, ct: "application/xml", body: xml}
	})
	out := e.Enrichment(Row{Type: "SCHEDULE 13G/A", Source: "SEC", URL: "https://www.sec.gov/Archives/edgar/data/1/2/xslSCHEDULE_13G_X02/primary_doc.xml"})
	seen := tr.documentURLs()
	if len(seen) == 0 {
		t.Fatal("no document was read")
	}
	if strings.Contains(seen[0], "xsl") {
		t.Errorf("the raw XML is read, not the rendered page: %q", seen[0])
	}
	if out == nil {
		t.Fatal("enrichment is nil")
	}
	if !strings.Contains(out.Subject, "2.4%") {
		t.Errorf("subject = %q", out.Subject)
	}
	if !strings.Contains(out.Subject, "Capital Ventures International") {
		t.Errorf("subject = %q", out.Subject)
	}
	if !strings.Contains(out.Summary, "2.4% of Quantum eMotion Corp.") {
		t.Errorf("summary = %q", out.Summary)
	}
}

func TestNonOwnershipFormsDeferToTheModel(t *testing.T) {
	e, _ := newStubbedEdgar(t, func(u string) stubReply {
		return stubReply{err: errors.New("unexpected fetch of " + u)}
	})
	if out := e.Enrichment(Row{Type: "6-K", Source: "SEC", URL: "https://www.sec.gov/x/y.htm"}); out != nil {
		t.Errorf("enrichment = %+v", out)
	}
}

func TestOfferingFormsAndAdministrativeFForms(t *testing.T) {
	cases := map[string]string{
		"F-1": Offerings, "F-10": Offerings, "S-1": Offerings, "424B5": Offerings,
		"F-X": Other, "F-N": Other, "25": Events, "6-K": Financials, "SCHEDULE 13G": Insider,
	}
	for form, want := range cases {
		if got := EdgarCategory(form); got != want {
			t.Errorf("EdgarCategory(%q) = %q, want %q", form, got, want)
		}
	}
}
