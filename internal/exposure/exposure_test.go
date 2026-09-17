package exposure

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"reflect"
	"strings"
	"sync"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/browserhttp"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func equal(t *testing.T, got, want any, msg string) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		if msg != "" {
			t.Errorf("%s: got %#v, want %#v", msg, got, want)
			return
		}
		t.Errorf("got %#v, want %#v", got, want)
	}
}

func almost(t *testing.T, got, want float64, msg string) {
	t.Helper()
	if math.IsNaN(got) || math.Abs(got-want) >= 5e-8 {
		if msg != "" {
			t.Errorf("%s: got %v, want %v", msg, got, want)
			return
		}
		t.Errorf("got %v, want %v", got, want)
	}
}

type page struct {
	text   string
	header http.Header
}

type stub struct {
	t      *testing.T
	mu     sync.Mutex
	urls   []string
	tmx    map[string]map[string]any
	pages  map[string]page
	fail   error
	forbid bool
}

func (s *stub) RoundTrip(req *http.Request) (*http.Response, error) {
	body := ""
	if req.Body != nil {
		b, _ := io.ReadAll(req.Body)
		req.Body.Close()
		body = string(b)
	}
	url := req.URL.String()
	s.mu.Lock()
	s.urls = append(s.urls, url)
	s.mu.Unlock()
	if s.fail != nil {
		return nil, s.fail
	}
	if req.Method == http.MethodPost && url == market.TMXURL {
		if s.forbid {
			s.t.Errorf("must not classify: %s", body)
			return nil, errors.New("must not classify")
		}
		return respond(req, s.tmxAnswer(body), nil), nil
	}
	if s.forbid {
		s.t.Errorf("must not look it through: %s", url)
		return nil, errors.New("must not look it through")
	}
	if p, ok := s.pages[url]; ok {
		return respond(req, p.text, p.header), nil
	}
	return nil, errors.New("no stub for " + url)
}

func (s *stub) tmxAnswer(body string) string {
	var q map[string]any
	_ = json.Unmarshal([]byte(body), &q)
	vars, _ := q["variables"].(map[string]any)
	sym := py.S(vars["symbol"])
	rec, ok := s.tmx[sym]
	if !ok {
		rec, ok = s.tmx[market.TMXBare(sym)]
	}
	var quote any
	if ok {
		quote = rec
	}
	out, _ := json.Marshal(map[string]any{"data": map[string]any{"getQuoteBySymbol": quote}})
	return string(out)
}

func (s *stub) harvestCalls() []string {
	s.mu.Lock()
	defer s.mu.Unlock()
	prefix := strings.Replace(HarvestPage, "%s/", "", 1)
	out := []string{}
	for _, u := range s.urls {
		if strings.HasPrefix(u, prefix) {
			out = append(out, strings.ToUpper(strings.Trim(strings.TrimPrefix(u, prefix), "/")))
		}
	}
	return out
}

func respond(req *http.Request, text string, header http.Header) *http.Response {
	if header == nil {
		header = http.Header{}
	}
	return &http.Response{Status: "200 OK", StatusCode: http.StatusOK, Proto: "HTTP/1.1", ProtoMajor: 1, ProtoMinor: 1, Header: header, Body: io.NopCloser(strings.NewReader(text)), ContentLength: int64(len(text)), Request: req}
}

func newClient(t *testing.T, s *stub) *Client {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	m := market.NewClient(st)
	s.t = t
	m.HTTP.Transport = s
	return NewClient(m)
}

func classified() map[string]map[string]any {
	return map[string]map[string]any{
		"RY":   {"name": "Royal Bank of Canada", "sector": "Financials", "industry": "Banking", "exchangeName": "Toronto Stock Exchange"},
		"SHOP": {"name": "Shopify Inc.", "sector": "Information Technology", "industry": "Software", "exchangeName": "Toronto Stock Exchange"},
		"PLTR": {"name": "Palantir Technologies Inc.", "sector": "Information Technology", "industry": "Software", "exchangeName": "Nasdaq Global Select"},
	}
}

func harvestURL(symbol string) string {
	return strings.Replace(HarvestPage, "%s", strings.ToLower(symbol), 1)
}

func harvestPage(rows [][3]string) string {
	var b strings.Builder
	b.WriteString("<table><tr><th>Name</th><th>Ticker</th><th>Weight</th></tr>")
	for _, r := range rows {
		b.WriteString("<tr><td>" + r[0] + "</td><td>" + r[1] + "</td><td>" + r[2] + "</td></tr>")
	}
	b.WriteString("</table>")
	return b.String()
}

func yahooURL(symbol, exchange, crumb string) string {
	return fmt.Sprintf(market.YahooSummaryURL, YahooSymbol(symbol, exchange), YahooHoldingsModule, crumb)
}

type yahooStub struct {
	mu    sync.Mutex
	urls  []string
	pages map[string]string
}

func (y *yahooStub) Get(rawURL string, headers map[string]string) (*browserhttp.Response, error) {
	y.mu.Lock()
	y.urls = append(y.urls, rawURL)
	y.mu.Unlock()
	if rawURL == market.YahooWarmURL {
		return &browserhttp.Response{Status: 200, URL: rawURL}, nil
	}
	if rawURL == market.YahooCrumbURL {
		return &browserhttp.Response{Status: 200, Body: []byte("crumb"), URL: rawURL}, nil
	}
	if text, ok := y.pages[rawURL]; ok {
		return &browserhttp.Response{Status: 200, Body: []byte(text), URL: rawURL}, nil
	}
	return &browserhttp.Response{Status: 404, URL: rawURL}, nil
}

func TestHoldingsAreSpreadByWeightAndTheRestIsUnclassified(t *testing.T) {
	c := newClient(t, &stub{tmx: classified()})
	rows := []Holding{
		{Ticker: "RY", Name: "", Weight: 60, Sector: "", Country: "", Exchange: "TSX", Currency: "CAD", Fund: false},
		{Ticker: "ZZZ", Name: "", Weight: 20, Sector: "", Country: "", Exchange: "", Currency: "", Fund: false},
		{Ticker: "SHOP", Name: "", Weight: 20, Sector: "Information Technology", Country: "Canada", Exchange: "", Currency: "", Fund: false},
	}
	agg := c.Lookthrough(rows, 0, nil)
	almost(t, agg.Sectors["Financials"], 0.6, "")
	almost(t, agg.Sectors["Information Technology"], 0.2, "")
	almost(t, agg.Countries["Canada"], 0.8, "")
	almost(t, agg.Coverage, 0.8, "ZZZ has no record: its fifth is unclassified")
}

func TestAFundHeldByAFundIsLookedThrough(t *testing.T) {
	s := &stub{tmx: classified(), pages: map[string]page{
		harvestURL("OUTER"): {text: harvestPage([][3]string{{"Harvest Inner Index ETF", "INNER CN", "50%"}, {"Palantir Technologies Inc.", "PLTR US", "50%"}})},
		harvestURL("INNER"): {text: harvestPage([][3]string{{"Royal Bank of Canada", "RY CN", "100%"}})},
	}}
	c := newClient(t, s)
	rec := c.FundExposure("OUTER", "Harvest Test Outer ETF", "TSX", 0, nil)
	if rec == nil {
		t.Fatal("no record for the outer fund")
	}
	almost(t, rec.Sectors["Financials"], 0.5, "")
	almost(t, rec.Sectors["Information Technology"], 0.5, "")
	almost(t, rec.Countries["Canada"], 0.5, "")
	almost(t, rec.Countries["United States"], 0.5, "")
	equal(t, rec.Coverage, 1.0, "")
	inner := c.Store.ExposureRecord("fund:INNER")
	if inner == nil {
		t.Fatal("the inner fund's record is kept for the next fund that holds it: no record under fund:INNER")
	}
	equal(t, inner.Coverage, 1.0, "the inner fund's record is kept for the next fund that holds it")
}

func TestAHoldingStatedWithSectorAndCountryIsTakenAsStated(t *testing.T) {
	c := newClient(t, &stub{forbid: true})
	rows := []Holding{{Ticker: "IBIT", Name: "iShares Bitcoin Trust ETF", Weight: 130.3, Sector: "Digital assets", Country: "United States", Exchange: "", Currency: "", Fund: true}}
	agg := c.Lookthrough(rows, 0, nil)
	equal(t, agg.Sectors, map[string]float64{"Digital assets": 1.0}, "")
	equal(t, agg.Countries, map[string]float64{"United States": 1.0}, "")
	equal(t, agg.Coverage, 1.0, "")
}

func TestAFundNamedWithoutATickerIsResolvedThenLookedThrough(t *testing.T) {
	tmx := classified()
	tmx["AAPL"] = map[string]any{"name": "Apple Inc.", "sector": "Information Technology", "industry": "Hardware", "exchangeName": "Nasdaq Global Select"}
	s := &stub{tmx: tmx, pages: map[string]page{harvestURL("APLE"): {text: harvestPage([][3]string{{"Apple Inc.", "AAPL US", "100%"}})}}}
	c := newClient(t, s)
	c.SymbolSearch = func(text string) []SearchMatch {
		return []SearchMatch{{Symbol: "APLE", Exchange: "TSX", Currency: "CAD"}}
	}
	rows := []Holding{{Ticker: "", Name: "Harvest Apple Enhanced High Income Shares ETF", Weight: 7.0, Sector: "", Country: "", Exchange: "", Currency: "", Fund: true}}
	agg := c.Lookthrough(rows, 0, nil)
	equal(t, s.harvestCalls(), []string{"APLE"}, "the fund is looked through under the ticker the directory gave")
	equal(t, agg.Sectors, map[string]float64{"Information Technology": 1.0}, "")
	equal(t, agg.Countries, map[string]float64{"United States": 1.0}, "")
}

func TestABareTickerAnsweredWithADepositaryReceiptIsRetriedAsTheUSListing(t *testing.T) {
	s := &stub{tmx: map[string]map[string]any{
		"PLTR":    {"name": "Palantir CDR (CAD Hedged)", "sector": "Technology", "industry": "Software", "exchangeName": "Toronto Stock Exchange"},
		"PLTR:US": {"name": "Palantir Technologies Inc.", "sector": "Technology", "industry": "Software", "exchangeName": "Nasdaq Global Select"},
	}}
	c := newClient(t, s)
	cl := c.ClassifyShare("PLTR", "", "")
	equal(t, [2]string{cl.Sector, cl.Country}, [2]string{"Information Technology", "United States"}, "")
}

func TestAFamilyWithoutAnAdapterFallsBackToYahoo(t *testing.T) {
	summary := `{"quoteSummary":{"result":[{"topHoldings":{"holdings":[{"symbol":"RY.TO","holdingName":"Royal Bank of Canada","holdingPercent":{"raw":1.0}}],"sectorWeightings":[{"financial_services":{"raw":1.0}}]}}]}}`
	s := &stub{tmx: classified()}
	c := newClient(t, s)
	y := &yahooStub{pages: map[string]string{yahooURL("ZZZ", "TSX", "crumb"): summary}}
	c.Market.Yahoo.Open = func() (market.YahooDoer, error) { return y, nil }
	rec := c.FundExposure("ZZZ", "Someone Else Global Equity ETF", "TSX", 0, nil)
	if rec == nil {
		t.Fatal("no record from the fallback")
	}
	equal(t, rec.Sectors, map[string]float64{"Financials": 1.0}, "the fund's stated sectors")
	equal(t, rec.Countries, map[string]float64{"Canada": 1.0}, "the countries from its named holdings")
	equal(t, rec.Source, "Yahoo Finance", "")
}

func TestAFundNoSourceCoversIsStoredAsUnclassified(t *testing.T) {
	c := newClient(t, &stub{fail: errors.New("down")})
	rec := c.RefreshSecurity(store.Security{ID: "sec-s-1", Symbol: "ZZZ", Name: "Nobody Fund ETF", PrimaryExchange: "TSX", Currency: "CAD"})
	if len(rec.Sectors) != 0 || len(rec.Countries) != 0 || rec.Coverage != 0.0 {
		t.Errorf("got sectors %v, countries %v, coverage %v, want none", rec.Sectors, rec.Countries, rec.Coverage)
	}
	stored := c.Store.ExposureRecord("sec-s-1")
	if stored == nil {
		t.Fatal("no record stored under sec-s-1")
	}
	equal(t, stored.Coverage, 0.0, "")
}

func TestAShareIsItsOneSectorAndCountry(t *testing.T) {
	c := newClient(t, &stub{tmx: classified()})
	rec := c.RefreshSecurity(store.Security{ID: "sec-s-ry", Symbol: "RY", Name: "Royal Bank of Canada", PrimaryExchange: "TSX", Currency: "CAD"})
	equal(t, rec.Sectors, map[string]float64{"Financials": 1.0}, "")
	equal(t, rec.Countries, map[string]float64{"Canada": 1.0}, "")
	equal(t, rec.Coverage, 1.0, "")
	equal(t, c.Stale([]string{"sec-s-ry", "sec-s-none"}), []string{"sec-s-none"}, "")
}
