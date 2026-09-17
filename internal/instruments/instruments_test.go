package instruments

import (
	"fmt"
	"io"
	"net/http"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type stubTransport func(*http.Request) (*http.Response, error)

func (f stubTransport) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }

func reply(status int, body string) *http.Response {
	return &http.Response{StatusCode: status, Status: fmt.Sprintf("%d %s", status, http.StatusText(status)), Header: http.Header{}, Body: io.NopCloser(strings.NewReader(body))}
}

func tempStore(t *testing.T) *store.Store {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	return st
}

func symbols(rows []Match) []string {
	out := []string{}
	for _, r := range rows {
		out = append(out, r.Symbol)
	}
	return out
}

func head(list []string, n int) []string {
	if len(list) > n {
		return list[:n]
	}
	return list
}

func TestAliasesFindWhatPeopleType(t *testing.T) {
	cases := []struct {
		text string
		n    int
		want []string
		msg  string
	}{
		{"WTI", 1, []string{"CL"}, ""},
		{"crude", 2, []string{"CL", "BZ"}, ""},
		{"NDX", 1, []string{"NDX"}, ""},
		{"nasdaq 100", 1, []string{"NDX"}, ""},
		{"VIX", 1, []string{"VIX"}, ""},
		{"volatility", 1, []string{"VIX"}, ""},
		{"gold", 1, []string{"GC"}, ""},
		{"ES", 1, []string{"ES"}, "the S&P 500 E-mini, the after-hours read"},
		{"futures", 4, []string{"ES", "NQ", "YM", "RTY"}, ""},
		{"dow futures", 1, []string{"YM"}, ""},
		{"nasdaq futures", 1, []string{"NQ"}, ""},
	}
	for _, c := range cases {
		if got := head(symbols(Search(c.text)), c.n); !reflect.DeepEqual(got, c.want) {
			t.Errorf("Search(%q)[:%d] = %v, want %v %s", c.text, c.n, got, c.want, c.msg)
		}
	}
	if es := Find("es", "cme"); es == nil || es.Yahoo != "ES=F" || KindLabel["Future"] != "Futures" {
		t.Errorf("Find(es, cme) = %+v, KindLabel[Future] = %q", es, KindLabel["Future"])
	}
	if got := Search("ZZZZ"); len(got) != 0 {
		t.Errorf("Search(ZZZZ) = %v, want none", got)
	}
	if got := Search("V"); len(got) != 0 {
		t.Errorf("a single letter is not a search for every V: %v", got)
	}
	if got := head(symbols(Search("VI")), 1); !reflect.DeepEqual(got, []string{"VIX"}) {
		t.Errorf("Search(VI)[:1] = %v, want [VIX]", got)
	}
	row := Search("VIX")[0]
	if row.Name != "CBOE Volatility Index" || row.Exchange != "Index" || row.Currency != "USD" || row.Kind != "Index" {
		t.Errorf("row = %+v", row)
	}
}

func TestFindBySymbolAndVenue(t *testing.T) {
	if r := Find("cl", "nymex"); r == nil || r.Yahoo != "CL=F" {
		t.Errorf("Find(cl, nymex) = %+v, want CL=F", r)
	}
	if r := Find("CL", "TSX"); r != nil {
		t.Errorf("a listing with the same letters is not the future: %+v", r)
	}
	if r := Find("SHOP", "TSX"); r != nil {
		t.Errorf("Find(SHOP, TSX) = %+v, want nil", r)
	}
}

func TestYahooMetaBecomesAQuote(t *testing.T) {
	text := `{"chart": {"result": [{"meta": {"regularMarketPrice": 99.4, "chartPreviousClose": 93.03, "currency": "USD", "shortName": "Crude Oil Oct 26", "exchangeName": "NYM"}}]}}`
	q := market.ParseYahooQuote(text)
	if q == nil {
		t.Fatal("no quote")
	}
	got := [6]any{py.Deref(q.Price, -1), py.Round(py.Deref(q.PriceChange, -1), 2), py.Round(py.Deref(q.PercentChange, -1), 2), py.Deref(q.PrevClose, -1), q.Currency, q.Name}
	if want := [6]any{99.4, 6.37, 6.85, 93.03, "USD", "Crude Oil Oct 26"}; got != want {
		t.Errorf("quote = %v, want %v", got, want)
	}
	if got := market.ParseYahooQuote(`{"chart": {"result": []}}`); got != nil {
		t.Errorf("empty result = %+v, want nil", got)
	}
}

func TestAnInstrumentIsQuotedFromYahooUnderItsOwnKey(t *testing.T) {
	c := market.NewClient(tempStore(t))
	needing := c.QuoteSymbolsNeedingRefresh([]market.Rec{{Symbol: "VIX", Exchange: "Index", Currency: "USD", Kind: "Instrument", Yahoo: "^VIX", QuoteKey: "VIX@INDEX"}}, time.Now().UTC(), market.QuoteRefreshMinutes)
	if want := []market.Needed{{Symbol: "VIX@INDEX", Source: "yahoo_quote", Key: "^VIX"}}; !reflect.DeepEqual(needing, want) {
		t.Errorf("needing = %+v, want %+v", needing, want)
	}
}

func TestPreviousCloseFromTheUSDMarketWhenThePairHasNone(t *testing.T) {
	now := time.Date(2026, 9, 11, 16, 0, 0, 0, time.UTC)
	st := tempStore(t)
	st.UpsertFXRates(map[string]float64{"2026-09-10": 1.38, "2026-09-11": 1.39})
	day := func(d int) int64 { return time.Date(2026, 9, d, 0, 0, 0, 0, time.UTC).Unix() }
	bars := fmt.Sprintf(`[[%d, 1, 1, 1, 76000.0, 1], [%d, 1, 1, 1, 77000.0, 1], [%d, 1, 1, 1, 78000.0, 1]]`, day(9), day(10), day(11))
	var mu sync.Mutex
	candles := []string{}
	spot := `{"data": {"amount": "107907.0", "base": "BTC", "currency": "CAD"}}`
	c := market.NewClient(st)
	c.Now = func() time.Time { return now }
	c.HTTP.Transport = stubTransport(func(r *http.Request) (*http.Response, error) {
		mu.Lock()
		defer mu.Unlock()
		u := r.URL.String()
		switch {
		case strings.Contains(u, "/candles"):
			product, _, _ := strings.Cut(strings.TrimPrefix(r.URL.Path, "/products/"), "/")
			candles = append(candles, product)
			return reply(200, bars), nil
		case strings.HasPrefix(u, "https://api.exchange.coinbase.com/products/"):
			if strings.TrimPrefix(r.URL.Path, "/products/") == "BTC-USD" {
				return reply(200, `{"id": "BTC-USD"}`), nil
			}
			return reply(404, "{}"), nil
		case strings.Contains(u, "/v2/prices/"):
			return reply(200, spot), nil
		}
		return reply(404, ""), nil
	})
	rec := c.FetchCoinbaseSpot("BTC-CAD", now)
	closeUSD, rate := 77000.0, 1.38
	wantPrev := closeUSD * rate
	wantPct := py.Round((107907.0-106260.0)/106260.0*100, 2)
	if rec == nil || py.Deref(rec.Price, -1) != 107907.0 || rec.PrevClose == nil || *rec.PrevClose != wantPrev || py.Round(py.Deref(rec.PercentChange, -1), 2) != wantPct {
		t.Errorf("yesterday's close, not today's running bar, converted to the pair's currency: %+v, want price 107907 prevClose %v percentChange %v", rec, wantPrev, wantPct)
	}
	if len(candles) == 0 || candles[0] != "BTC-USD" {
		t.Errorf("candles asked of %v, want BTC-USD", candles)
	}
	c.FetchCoinbaseSpot("BTC-CAD", now)
	if len(candles) != 1 {
		t.Errorf("the previous close is remembered for the day: candles asked %d times", len(candles))
	}
	mu.Lock()
	spot = `{"data": {"amount": "2.0", "currency": "CAD"}}`
	mu.Unlock()
	rec = c.FetchCoinbaseSpot("XYZ-CAD", now)
	if rec == nil || py.Deref(rec.Price, -1) != 2.0 || rec.Currency != "CAD" || rec.PrevClose != nil || rec.PriceChange != nil || rec.PercentChange != nil {
		t.Errorf("no market, no change: the price alone: %+v", rec)
	}
}

func TestTheDirectoryFindsThemByTheWordsPeopleType(t *testing.T) {
	if got := symbols(Search("fed")); !reflect.DeepEqual(got, []string{"ZQ"}) {
		t.Errorf("Search(fed) = %v, want [ZQ]", got)
	}
	if got := symbols(Search("sofr")); !reflect.DeepEqual(got, []string{"SR3"}) {
		t.Errorf("Search(sofr) = %v, want [SR3]", got)
	}
	if got := symbols(Search("fed funds")); !reflect.DeepEqual(got, []string{"ZQ"}) {
		t.Errorf("Search(fed funds) = %v, want [ZQ]", got)
	}
	if got := Label("ZQ"); got != "FED FUNDS" {
		t.Errorf("Label(ZQ) = %q, want FED FUNDS", got)
	}
}

func TestTheRateIsThePriceTakenFromAHundredAndNothingElseCarriesOne(t *testing.T) {
	if r := ImpliedRate("ZQ", py.Ptr(96.13)); r == nil || *r != 3.87 {
		t.Errorf("ImpliedRate(ZQ, 96.13) = %v, want 3.87", py.Deref(r, -1))
	}
	if r := ImpliedRate("SR3", py.Ptr(95.765)); r == nil || *r != 4.235 {
		t.Errorf("ImpliedRate(SR3, 95.765) = %v, want 4.235", py.Deref(r, -1))
	}
	if r := ImpliedRate("ES", py.Ptr(7674.0)); r != nil {
		t.Errorf("an index future prices no rate: %v", *r)
	}
	if r := ImpliedRate("ZQ", nil); r != nil {
		t.Errorf("and an unquoted contract prices none either: %v", *r)
	}
}
