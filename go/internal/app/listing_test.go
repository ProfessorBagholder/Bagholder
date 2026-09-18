package app

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/model"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const (
	whenEarlier = "2026-01-05T14:40:00+00:00"
	whenFill    = "2026-03-02T14:31:00+00:00"
	whenLater   = "2026-04-09T15:02:00+00:00"
)

type quoteStub struct {
	price   float64
	percent float64
}

func (q quoteStub) RoundTrip(req *http.Request) (*http.Response, error) {
	body := map[string]any{"data": map[string]any{"getQuoteBySymbol": map[string]any{"price": q.price, "percentChange": q.percent, "priceChange": 0.0}}}
	raw, _ := json.Marshal(body)
	if req.Body != nil {
		_, _ = io.Copy(io.Discard, req.Body)
	}
	return &http.Response{StatusCode: 200, Status: "200 OK", Header: http.Header{"Content-Type": []string{"application/json"}}, Body: io.NopCloser(strings.NewReader(string(raw))), Request: req}, nil
}

func withQuote(a *App, price, percent float64) {
	a.mk.HTTP = &http.Client{Transport: quoteStub{price: price, percent: percent}}
}

func at(when string) func(*store.Activity) {
	return func(act *store.Activity) { act.OccurredAt = when }
}

func security(t *testing.T, a *App, id, symbol, name, exchange, currency string) {
	t.Helper()
	a.st.UpsertSecurities([]store.Security{{ID: id, Symbol: symbol, Name: name, PrimaryExchange: exchange, Currency: currency}})
}

func rows(t *testing.T, a *App, acts ...store.Activity) {
	t.Helper()
	a.st.MergeLocalRows(acts)
	a.invalidate(true)
}

func fillWhens(t *testing.T, out map[string]any) []string {
	t.Helper()
	fills, ok := out["fills"].([]model.Fill)
	if !ok {
		t.Fatalf("fills: %#v", out["fills"])
	}
	when := []string{}
	for _, f := range fills {
		when = append(when, f.When)
	}
	return when
}

func TestAListingTheBookHoldsAnswersWithTheHoldingWhosePageItIs(t *testing.T) {
	a := newTestApp(t)
	withQuote(a, 1.25, -2.0)
	security(t, a, "sec-qnc", "QNC", "Quantum eMotion Corp", "TSX-V", "CAD")
	rows(t, a, fixtures.Buy("b1", "QNC", 10, 5.0, "2026-03-02", fixtures.WithSecurity("sec-qnc"), at(whenFill)))
	held := a.model.Base().Positions
	if len(held) != 1 {
		t.Fatalf("the book holds one position, got %d", len(held))
	}
	out := a.listingPayload("QNC", "TSX-V", "", "")
	if out["ok"] != true || out["positionId"] != held[0].ID {
		t.Errorf("(ok, positionId) = (%v, %v), want (true, %q)", out["ok"], out["positionId"], held[0].ID)
	}
}

func TestAListingTradedBeforeCarriesTheExecutionsOfThoseTradesInTime(t *testing.T) {
	a := newTestApp(t)
	withQuote(a, 1.8, 1.5)
	security(t, a, "sec-qnc", "QNC", "Quantum eMotion Corp", "TSX-V", "CAD")
	security(t, a, "sec-qnc-us", "QNC", "Quantum eMotion Corp", "NYSE", "USD")
	security(t, a, "sec-qnc-opt", "QNC 16JAN26 5.00 CALL", "Quantum eMotion Corp", "TSX-V", "CAD")
	rows(t, a,
		fixtures.Buy("b0", "QNC", 4, 4.0, "2026-01-05", fixtures.WithSecurity("sec-qnc"), at(whenEarlier)),
		fixtures.Buy("b1", "QNC", 10, 5.0, "2026-03-02", fixtures.WithSecurity("sec-qnc"), at(whenFill)),
		fixtures.Sell("s1", "QNC", 14, 6.5, "2026-04-09", fixtures.WithSecurity("sec-qnc"), at(whenLater)),
		fixtures.BTC("o1", "QNC 16JAN26 5.00 CALL", 1, 1.1, "2026-02-02", "BUYTOOPEN", fixtures.WithSecurity("sec-qnc-opt"), at("2026-02-02T14:00:00+00:00")),
		fixtures.Buy("u1", "QNC", 9, 2.2, "2026-02-03", fixtures.WithSecurity("sec-qnc-us"), fixtures.WithCurrency("USD"), fixtures.WithAccount("US", "acct-2"), at("2026-02-03T14:00:00+00:00")),
	)
	out := a.listingPayload("QNC", "TSX-V", "", "")
	if out["positionId"] != nil {
		t.Errorf("positionId = %v, want none", out["positionId"])
	}
	if got := fillWhens(t, out); !equalStrings(got, []string{whenEarlier, whenFill, whenLater}) {
		t.Errorf("fills = %v; the listing's own trades, oldest first; an option is not the share, and another venue is another listing", got)
	}
	if got := []any{out["name"], out["exchange"], out["currency"], out["kind"]}; got[0] != "Quantum eMotion Corp" || got[1] != "TSX-V" || got[2] != "CAD" || got[3] != "Shares" {
		t.Errorf("(name, exchange, currency, kind) = %v", got)
	}
	price, _ := out["price"].(*float64)
	pct, _ := out["percentChange"].(*float64)
	if price == nil || *price != 1.8 || pct == nil || *pct != 1.5 {
		t.Errorf("(price, percentChange) = (%v, %v), want (1.8, 1.5)", out["price"], out["percentChange"])
	}
}

func TestAListingNeverTradedIsNamedByTheWatchlistAndHasNoExecutions(t *testing.T) {
	a := newTestApp(t)
	withQuote(a, 0.265, 0.0)
	a.st.AddWatch("YES", "TSX-V", "Char Technologies Ltd.", "CAD", "", "")
	a.invalidate(true)
	out := a.listingPayload("YES", "TSX-V", "", "")
	if got := fillWhens(t, out); len(got) != 0 {
		t.Errorf("fills = %v, want none", got)
	}
	if out["name"] != "Char Technologies Ltd." || out["currency"] != "CAD" {
		t.Errorf("(name, currency) = (%v, %v)", out["name"], out["currency"])
	}
}

func TestAListingTheBookHasNeverSeenAnswersWithWhatWasAskedFor(t *testing.T) {
	a := newTestApp(t)
	withQuote(a, 284.21, -0.34)
	out := a.listingPayload("RY", "TSX", "CAD", "Royal Bank of Canada")
	if out["ok"] != true || out["symbol"] != "RY" || out["exchange"] != "TSX" || out["name"] != "Royal Bank of Canada" || len(fillWhens(t, out)) != 0 {
		t.Errorf("(ok, symbol, exchange, name, fills) = (%v, %v, %v, %v, %v)", out["ok"], out["symbol"], out["exchange"], out["name"], out["fills"])
	}
	price, _ := out["price"].(*float64)
	if price == nil || *price != 284.21 {
		t.Errorf("price = %v, want 284.21", out["price"])
	}
}

func TestATickerWithNoVenueMatchesTheBookWhateverVenueItHoldsItOn(t *testing.T) {
	a := newTestApp(t)
	withQuote(a, 1.25, -2.0)
	security(t, a, "sec-shop", "SHOP.TO", "Shopify Inc.", "TSX", "CAD")
	rows(t, a,
		fixtures.Buy("b1", "SHOP.TO", 10, 5.0, "2026-03-02", fixtures.WithSecurity("sec-shop"), at(whenFill)),
		fixtures.Sell("s1", "SHOP.TO", 10, 6.5, "2026-04-09", fixtures.WithSecurity("sec-shop"), at(whenLater)),
	)
	out := a.listingPayload("SHOP", "", "", "")
	if out["symbol"] != "SHOP" || out["exchange"] != "TSX" || out["name"] != "Shopify Inc." {
		t.Errorf("(symbol, exchange, name) = (%v, %v, %v)", out["symbol"], out["exchange"], out["name"])
	}
	if got := fillWhens(t, out); len(got) != 2 {
		t.Errorf("fills = %v, want the listing's own two", got)
	}
}

func TestATickerThatIsNotOneIsRefused(t *testing.T) {
	a := newTestApp(t)
	if out := a.listingPayload("  ", "", "", ""); out["ok"] != false {
		t.Errorf("ok = %v, want false", out["ok"])
	}
}

func equalStrings(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}
