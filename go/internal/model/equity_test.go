package model

import (
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func yearsByName(rows []YearRow) map[string]YearRow {
	out := map[string]YearRow{}
	for _, y := range rows {
		out[y.Year] = y
	}
	return out
}

func TestReturnsAndDrawdown(t *testing.T) {
	v := BuildView(viewBase(), nil)
	years := yearsByName(v.Years)
	almost(t, years["2025"].R, (1+0.3)*(1+100.0/1500)-1, "2025 return")
	almost(t, deref(years["2025"].SpR), 0.2, "2025 index return")
	almost(t, deref(years["2025"].Flow), 200, "2025 flow")
	almost(t, years["2026"].R, (1400.0-1600)/1600, "2026 return")
	almost(t, deref(years["2026"].SpR), 0.05, "2026 index return")
	dd := v.Equity.Drawdown
	almost(t, deref(dd.Pct), (1400.0-1600)/1600, "drawdown pct")
	if dd.At != "2026-03-31" {
		t.Errorf("drawdown at: %q", dd.At)
	}
	if v.Equity.Annualized.Rate == nil {
		t.Error("annualized rate is nil")
	}
}

func TestDrawdownIgnoresWithdrawalsAndDeposits(t *testing.T) {
	series := EquitySeries([]store.NavPoint{
		{Date: "2026-01-01", Equity: 100000, NetDeposits: ptr(100000)},
		{Date: "2026-01-02", Equity: 101000, NetDeposits: ptr(100000)},
		{Date: "2026-01-03", Equity: 21000, NetDeposits: ptr(20000)},
		{Date: "2026-01-04", Equity: 21210, NetDeposits: ptr(20000)},
		{Date: "2026-01-05", Equity: 41210, NetDeposits: ptr(40000)},
		{Date: "2026-01-06", Equity: 37089, NetDeposits: ptr(40000)},
	})
	dd := DrawdownOf(series)
	almostPlaces(t, deref(dd.Pct), -0.1, 4, "drawdown pct")
	if dd.At != "2026-01-06" {
		t.Errorf("drawdown at: %q", dd.At)
	}
	if dd.PeakAt != "2026-01-05" {
		t.Errorf("peak at: %q", dd.PeakAt)
	}
	almostDelta(t, deref(dd.Abs), -4121, 1, "drawdown abs")
}

func TestNegligibleYearsAreSkipped(t *testing.T) {
	series := EquitySeries([]store.NavPoint{
		{Date: "2020-12-22", Equity: 0, NetDeposits: ptr(0)},
		{Date: "2020-12-23", Equity: 100, NetDeposits: ptr(100)},
		{Date: "2020-12-31", Equity: 101, NetDeposits: ptr(100)},
		{Date: "2023-06-30", Equity: 45000, NetDeposits: ptr(40000)},
		{Date: "2023-12-31", Equity: 50000, NetDeposits: ptr(40000)},
		{Date: "2024-12-31", Equity: 60000, NetDeposits: ptr(40000)},
	})
	years := YearlyReturns(series, map[string]float64{}, "2025-01-01")
	names := []string{}
	for _, y := range years {
		names = append(names, y.Year)
	}
	if !reflect.DeepEqual(names, []string{"2023", "2024"}) {
		t.Fatalf("years: %v", names)
	}
	if years[0].From != "2023-06-30" {
		t.Errorf("2023 is measured from the first funded point, not from the $101 of 2020: %q", years[0].From)
	}
	almostPlaces(t, years[0].R, 50000.0/45000-1, 6, "2023 return")
}

func TestYearStartsWhereTheAccountWasReallyFunded(t *testing.T) {
	series := EquitySeries([]store.NavPoint{
		{Date: "2023-09-06", Equity: 0, NetDeposits: ptr(15)},
		{Date: "2023-09-13", Equity: 1666, NetDeposits: ptr(1682)},
		{Date: "2023-09-20", Equity: 111771, NetDeposits: ptr(112806)},
		{Date: "2023-10-04", Equity: 116832, NetDeposits: ptr(119270)},
		{Date: "2023-12-27", Equity: 135232, NetDeposits: ptr(125411)},
		{Date: "2024-06-30", Equity: 174611, NetDeposits: ptr(125411)},
		{Date: "2024-12-31", Equity: 170000, NetDeposits: ptr(125411)},
	})
	yr := yearReturn(series, "2023", "2025-01-01")
	if yr == nil {
		t.Fatal("no 2023 return")
	}
	if yr.from != "2023-09-20" {
		t.Errorf("from: %q", yr.from)
	}
	almostDelta(t, yr.r, (116832.0-111771-6464)/111771, 0.2, "2023 return")
	if !(yr.r > 0.05) {
		t.Errorf("2023 return %v is not above 5%%", yr.r)
	}
	if !(yr.r < 0.15) {
		t.Errorf("2023 return %v is not below 15%%", yr.r)
	}
	bench := map[string]float64{"2022-12-30": 3800.0, "2023-09-19": 4400.0, "2023-12-29": 4770.0, "2024-12-31": 5880.0}
	by := yearsByName(YearlyReturns(series, bench, "2025-01-01"))
	almostPlaces(t, deref(by["2023"].SpR), 4770.0/4400-1, 6, "the index is measured over the same span as the account")
	almostPlaces(t, deref(by["2024"].SpR), 5880.0/4770-1, 6, "2024 index return")
}

func TestYearlyReturnsCompareAgainstTheChosenIndex(t *testing.T) {
	market := &store.MarketData{
		FX:         map[string]float64{},
		Benchmark:  map[string]float64{"2023-12-29": 100.0, "2024-12-31": 110.0},
		Benchmarks: map[string]map[string]float64{"SP500": {"2023-12-29": 100.0, "2024-12-31": 110.0}, "TSX": {"2023-12-29": 200.0, "2024-12-31": 250.0}},
	}
	snapshot := &store.Snapshot{NavHistory: []store.NavPoint{{Date: "2023-12-31", Equity: 100000, NetDeposits: ptr(100000)}, {Date: "2024-12-31", Equity: 120000, NetDeposits: ptr(100000)}}}
	base := BuildBase(snapshot, market, noJournal(), "2025-01-01", nil)
	if got := CleanFilters(map[string]any{"benchmark": "tsx"}).Benchmark; got != "TSX" {
		t.Errorf("tsx: %q", got)
	}
	if got := CleanFilters(map[string]any{"benchmark": "tsx60"}).Benchmark; got != "TSX60" {
		t.Errorf("tsx60: %q", got)
	}
	if got := CleanFilters(map[string]any{"benchmark": "nope"}).Benchmark; got != "SP500" {
		t.Errorf("nope: %q", got)
	}
	v := BuildView(base, map[string]any{})
	if v.Benchmark != (BenchmarkView{"SP500", "S&P 500"}) {
		t.Errorf("benchmark: %+v", v.Benchmark)
	}
	almostPlaces(t, deref(v.Years[len(v.Years)-1].SpR), 0.10, 6, "S&P 500 return")
	v = BuildView(base, map[string]any{"benchmark": "TSX"})
	if v.Benchmark != (BenchmarkView{"TSX", "S&P/TSX"}) {
		t.Errorf("benchmark: %+v", v.Benchmark)
	}
	almostPlaces(t, deref(v.Years[len(v.Years)-1].SpR), 0.25, 6, "TSX return")
	market.Benchmarks["TSX60"] = map[string]float64{"2023-12-29": 100.0, "2024-12-31": 115.0}
	base = BuildBase(snapshot, market, noJournal(), "2025-01-01", nil)
	v = BuildView(base, map[string]any{"benchmark": "TSX60"})
	if v.Benchmark != (BenchmarkView{"TSX60", "TSX 60"}) {
		t.Errorf("benchmark: %+v", v.Benchmark)
	}
	almostPlaces(t, deref(v.Years[len(v.Years)-1].SpR), 0.15, 6, "TSX 60 return")
	almostPlaces(t, v.Years[len(v.Years)-1].R, 0.20, 6, "the account's own return does not depend on the index")
}
