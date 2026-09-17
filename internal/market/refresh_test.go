package market

import (
	"strings"
	"testing"
	"time"
)

func bocAndFred(offlineTMX func(call stubCall) (int, string, error)) func(call stubCall) (int, string, error) {
	boc := jsonText(obj("observations", []any{obj("d", "2026-09-04", "FXUSDCAD", obj("v", "1.38"))}))
	fred := "observation_date,SP500\n2026-09-04,7000\n"
	return func(call stubCall) (int, string, error) {
		switch {
		case strings.HasPrefix(call.URL, BocURL):
			return 200, boc, nil
		case call.URL == FredURL:
			return 200, fred, nil
		case call.URL == TMXURL && offlineTMX != nil:
			return offlineTMX(call)
		}
		return 0, "", errNoRoute
	}
}

func TestPeriodicRefreshPacesFXAndBenchmarkAndRefetchesStaleRecords(t *testing.T) {
	t0 := utc(2026, 9, 6, 12, 0, 0)
	c, s := newTestClient(t, t0)
	syms := []Rec{rec("CCHI", "TSX", "CAD", "")}
	s.set(bocAndFred(func(call stubCall) (int, string, error) {
		p, _ := parseTMX(call.Body)
		switch p.Op {
		case "getQuoteBySymbol":
			return 200, jsonText(obj("data", obj("getQuoteBySymbol", obj("price", 1.0)))), nil
		case "getDividendsForSymbol":
			return 200, jsonText(obj("data", obj("dividends", obj("dividends", []any{obj("exDate", "2026-09-01", "payableDate", "2026-09-05", "amount", 0.1, "currency", "CAD")})))), nil
		}
		return 0, "", errNoRoute
	}))
	divs := func() int { return len(s.tmxSymbols("getDividendsForSymbol")) }
	out := c.RefreshPeriodic(syms, t0)
	eq(t, [3]int{out.FX, out.Benchmark, out.Distributions}, [3]int{1, 1, 1}, "")
	eq(t, [2]int{s.gets(), divs()}, [2]int{2, 1}, "")
	s.reset()
	c.RefreshPeriodic(syms, t0.Add(5*time.Minute))
	eq(t, s.gets(), 2, "a missing index does not wait for the clock")
	c.Store.UpsertBenchmarkPrices(map[string]float64{"2026-09-04": 1500.0}, "TSX")
	c.Store.UpsertBenchmarkPrices(map[string]float64{"2026-09-04": 1500.0}, "TSX60")
	s.reset()
	out = c.RefreshPeriodic(syms, t0.Add(10*time.Minute))
	eq(t, [3]int{out.FX, out.Benchmark, out.Distributions}, [3]int{0, 0, 0}, "")
	eq(t, [2]int{s.gets(), divs()}, [2]int{0, 0}, "")
	s.reset()
	c.RefreshPeriodic(syms, t0.Add(7*time.Hour))
	eq(t, [2]int{s.gets(), divs()}, [2]int{2, 0}, "")
	s.reset()
	c.RefreshPeriodic(syms, t0.Add(25*time.Hour))
	eq(t, divs(), 1, "")
	c.Store.SetMeta("market_attempt_at", "2026-09-08T19:50:00Z")
	s.reset()
	c.RefreshPeriodic(syms, utc(2026, 9, 8, 20, 0, 0))
	eq(t, s.gets(), 0, "")
	s.reset()
	c.RefreshPeriodic(syms, utc(2026, 9, 8, 20, 45, 0))
	eq(t, s.gets(), 2, "")
	eq(t, c.FXDayPublishedButMissing(utc(2026, 9, 12, 21, 0, 0)), false, "Saturday: nothing to publish")
}

func TestTSXCompositeIsFetchedFromTMXAndStoredBySymbol(t *testing.T) {
	c, s := newTestClient(t, time.Time{})
	tmx := tmxSeries(tmxRow("2026-09-04", 1, 1, 1, 36513.8, 0), tmxRow("2026-09-03", 1, 1, 1, 36633.12, 0))
	s.set(func(call stubCall) (int, string, error) {
		if call.URL == TMXURL {
			return 200, tmx, nil
		}
		return 0, "", errNoRoute
	})
	eq(t, c.RefreshTSX(), 4, "two days for each of the two indices")
	starts := func() []string {
		out := []string{}
		for _, p := range s.tmxPosts() {
			out = append(out, p.Var("start"))
		}
		return uniqSorted(out)
	}
	eq(t, s.tmxSymbols("getTimeSeriesData"), []string{"^TSX", "^TX60"}, "the Composite and the 60")
	eq(t, starts(), []string{"2016-01-01"}, "first fetch goes back to 2016")
	eq(t, c.Store.BenchmarkPrices("TSX")["2026-09-04"], 36513.8, "")
	eq(t, c.Store.BenchmarkPrices("TSX60")["2026-09-04"], 36513.8, "")
	eq(t, c.Store.BenchmarkPrices("SP500"), map[string]float64{}, "kept apart from the S&P 500")
	eq(t, keysOf(c.Store.MarketData().Benchmarks), []string{"SP500", "TSX", "TSX60"}, "")
	eq(t, c.BenchmarkStale(utc(2026, 9, 5, 0, 0, 0)), true, "the S&P 500 has no closes yet")
	c.Store.UpsertBenchmarkPrices(map[string]float64{"2026-09-04": 6500.0}, "SP500")
	eq(t, c.BenchmarkStale(utc(2026, 9, 5, 0, 0, 0)), false, "")
	eq(t, c.BenchmarkStale(utc(2026, 10, 5, 0, 0, 0)), true, "closes older than the stale window")
	s.reset()
	c.RefreshTSX()
	eq(t, starts(), []string{"2026-08-28"}, "later fetches start a week before the newest stored day")
}

func TestRefreshUsesStoreAndSurvivesErrors(t *testing.T) {
	c, s := newTestClient(t, time.Time{})
	eq(t, c.RefreshAll(nil), RefreshResult{FX: 0, Benchmark: 0, Distributions: 0, Skipped: false}, "")
	eq(t, c.IsStale(time.Now().UTC(), nil), true, "")
	s.set(bocAndFred(nil))
	out := c.RefreshAll(nil)
	eq(t, out.FX, 1, "")
	eq(t, out.Benchmark, 1, "")
	eq(t, c.Store.FXRates(), map[string]float64{"2026-09-04": 1.38}, "")
}

func TestEveryRequestRecordsItsOutcomeAndAnEmptyChartSaysWhy(t *testing.T) {
	c, s := newTestClient(t, utc(2026, 9, 7, 12, 0, 0))
	r := rec("CH", "TSX-V", "CAD", "Shares")
	s.set(func(call stubCall) (int, string, error) {
		if strings.Contains(call.URL, "tmx") {
			return 0, "", errNoRoute
		}
		return 429, "Too Many Requests", nil
	})
	c.SetYahooBackoff(time.Time{})
	bars, _ := c.FetchHistory(r, "2026-02-01", "2026-02-10")
	eq(t, len(bars), 0, "")
	reason := c.ChartReason(r, "1d")
	if !strings.Contains(reason, "TMX Money could not be reached") {
		t.Fatalf("reason %q", reason)
	}
	if !strings.Contains(reason, "Yahoo Finance refused the request (too many)") {
		t.Fatalf("reason %q", reason)
	}
	health := map[string]Health{}
	for _, h := range c.SourceHealth() {
		health[h.Key] = h
	}
	eq(t, health["tmx"].OK, false, "")
	eq(t, health["tmx"].Error, "could not be reached", "")
	eq(t, health["yahoo"].OK, false, "")
	if !strings.HasPrefix(health["yahoo"].Error, "refused the request") {
		t.Fatal(health["yahoo"].Error)
	}
	c.SetYahooBackoff(time.Time{})
	s.set(func(call stubCall) (int, string, error) {
		if call.URL == TMXURL {
			return 200, jsonText(obj("data", obj("getTimeSeriesData", []any{}, "getQuoteBySymbol", nil))), nil
		}
		return 200, jsonText(obj("chart", obj("result", []any{}))), nil
	})
	c.FetchHistory(r, "2026-02-01", "2026-02-10")
	eq(t, c.ChartReason(r, "1d"), "No bars for this span from TMX Money or Yahoo Finance.", "")
	c.ResetHealth()
	s.set(func(call stubCall) (int, string, error) { return 404, "Not Found", nil })
	_, err := c.GetText("https://query1.finance.yahoo.com/v8/finance/chart/GONE.CN", nil)
	eq(t, StatusOf(err), 404, "")
	eq(t, c.SourceHealth(), []Health{}, "a missing symbol leaves the source's health alone")
	c.NoteSource("tmx", true, nil)
	got := [][2]any{}
	for _, h := range c.SourceHealth() {
		got = append(got, [2]any{h.Name, h.OK})
	}
	eq(t, got, [][2]any{{"TMX Money", true}}, "")
}
