package model

import (
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var tradeKeys = []string{"id", "symbol", "kind", "currency", "side", "qty", "mult", "entry", "exit", "entryDate", "exitDate", "holdDays", "pnl", "pnlCad", "pnlPct", "status", "fees", "account", "exchange", "grade", "tags"}
var kpiKeys = []string{"count", "wins", "losses", "breakeven", "winRate", "realized", "expectancy", "profitFactor", "avgHold", "avgWin", "avgLoss", "grossWin", "grossLoss"}
var positionKeys = []string{"id", "symbol", "kind", "currency", "account", "exchange", "qty", "avg", "cost", "held", "alloc", "short", "dayChange", "grade"}
var portfolioKeys = []string{"marketValue", "costBasis", "unrealized", "unrealizedPct", "positionCount", "accountCount", "nav", "navAccounts", "marginUsed", "marginUsedBy", "marginUsedPct", "availableMargin", "availableMarginUnavailable", "hasMargin", "cash", "cashPct", "dayChange", "dayChangePct"}
var allocationKeys = []string{"id", "symbol", "account", "value", "share"}
var yearKeys = []string{"year", "r", "days", "from", "to", "flow", "endV", "spR"}
var monthKeys = []string{"key", "label", "value", "count"}
var symbolKeys = []string{"symbol", "pnl", "n", "legs", "winRate", "avgHold"}
var queueKeys = []string{"id", "symbol", "date", "pnl", "missing"}
var holdingKeys = []string{"symbol", "qty", "per", "freq", "freqVerified", "annual", "yoc", "ytd", "ttm", "all", "nextExDate", "nextPayDate", "exPast", "payPast"}
var tileKeys = []string{"label", "total", "perMonth", "count", "yield", "projected", "earned", "book", "marginUsed", "interestPerMonth", "interestMonths"}

type caseDoc struct {
	Today    string                        `json:"today"`
	Snapshot store.Snapshot                `json:"snapshot"`
	Market   caseMarket                    `json:"market"`
	Filters  any                           `json:"filters"`
	Journal  map[string]store.JournalEntry `json:"journal"`
	Expect   map[string]any                `json:"expect"`
}

type caseMarket struct {
	FX            map[string]float64              `json:"fx"`
	Benchmark     map[string]float64              `json:"benchmark"`
	Benchmarks    map[string]map[string]float64   `json:"benchmarks"`
	Distributions map[string][]store.Distribution `json:"distributions"`
	Quotes        map[string]store.Quote          `json:"quotes"`
}

func pick(d map[string]any, keys []string) map[string]any {
	out := map[string]any{}
	for _, k := range keys {
		if v, ok := d[k]; ok {
			out[k] = v
		}
	}
	return out
}

func toMap(v any) map[string]any {
	b, _ := json.Marshal(v)
	var out map[string]any
	json.Unmarshal(b, &out)
	return out
}

func toList(v any) []map[string]any {
	b, _ := json.Marshal(v)
	var out []map[string]any
	json.Unmarshal(b, &out)
	return out
}

func fillSubs(fills []Fill) []any {
	fs := append([]Fill{}, fills...)
	sort.SliceStable(fs, func(i, j int) bool { return fs[i].When < fs[j].When })
	out := []any{}
	for _, f := range fs {
		out = append(out, f.Sub)
	}
	return out
}

func ExpectFrom(snap *store.Snapshot, market *store.MarketData, today string, filters any, journal map[string]store.JournalEntry) map[string]any {
	if journal == nil {
		journal = map[string]store.JournalEntry{}
	}
	base := BuildBase(snap, market, journal, today, nil)
	view := BuildView(base, filters)
	trades := append([]*Trade{}, view.Trades...)
	sort.SliceStable(trades, func(i, j int) bool {
		a, b := trades[i], trades[j]
		if a.EntryDate != b.EntryDate {
			return a.EntryDate < b.EntryDate
		}
		if a.ExitDate != b.ExitDate {
			return a.ExitDate < b.ExitDate
		}
		return a.Symbol < b.Symbol
	})
	positions := append([]*Position{}, view.Positions...)
	sort.SliceStable(positions, func(i, j int) bool {
		a, b := positions[i], positions[j]
		if a.Symbol != b.Symbol {
			return a.Symbol < b.Symbol
		}
		return a.Account < b.Account
	})
	cf := view.Cashflow
	tl := []any{}
	for _, t := range trades {
		row := pick(toMap(t.TradeCore), tradeKeys)
		row["fills"] = fillSubs(t.Fills)
		tl = append(tl, row)
	}
	pl := []any{}
	for _, p := range positions {
		row := pick(toMap(p.PositionCore), positionKeys)
		row["fills"] = fillSubs(p.Fills)
		pl = append(pl, row)
	}
	pf := pick(toMap(view.Portfolio), portfolioKeys)
	alloc := []any{}
	for _, a := range toList(view.Portfolio.Allocation) {
		alloc = append(alloc, pick(a, allocationKeys))
	}
	pf["allocation"] = alloc
	series := []any{}
	for _, p := range view.Equity.Series {
		series = append(series, map[string]any{"d": p.D, "v": p.V})
	}
	pickList := func(v any, keys []string) []any {
		out := []any{}
		for _, r := range toList(v) {
			out = append(out, pick(r, keys))
		}
		return out
	}
	holdings := append([]CashHolding{}, cf.Holdings...)
	sort.SliceStable(holdings, func(i, j int) bool { return holdings[i].Symbol < holdings[j].Symbol })
	opts := toMap(view.Options)
	return map[string]any{
		"kpi":              pick(toMap(view.KPI), kpiKeys),
		"trades":           tl,
		"positions":        pl,
		"positionsSummary": toMap(view.PositionsSummary),
		"portfolio":        pf,
		"equity":           map[string]any{"label": view.Equity.Label, "series": series, "drawdown": toMap(view.Equity.Drawdown), "annualized": toMap(view.Equity.Annualized)},
		"years":            pickList(view.Years, yearKeys),
		"benchmark":        toMap(view.Benchmark),
		"monthly":          pickList(view.Monthly, monthKeys),
		"bySymbol":         pickList(view.BySymbol, symbolKeys),
		"grades":           map[string]any{"buckets": pickList(view.Grades.Buckets, []string{"grade", "n", "pnl"}), "ungraded": view.Grades.Ungraded, "graded": view.Grades.Graded},
		"queue":            pickList(view.Queue, queueKeys),
		"options":          pick(opts, []string{"accounts", "symbols", "tags", "exchanges", "kinds", "years"}),
		"cashflowHoldings": pickList(holdings, holdingKeys),
		"cashflowTiles":    pickList(cf.Tiles, tileKeys),
		"cashflowMonths":   pickList(cf.Months, monthKeys),
		"cashflowTotal":    cf.Total, "cashflowCount": cf.Count, "cashflowSkipped": cf.SkippedFilters,
	}
}

func diffJSON(path string, want, got any, out *[]string) {
	switch w := want.(type) {
	case map[string]any:
		g, ok := got.(map[string]any)
		if !ok {
			*out = append(*out, path+": want object, got "+py.S(got))
			return
		}
		for k, wv := range w {
			gv, ok := g[k]
			if !ok {
				*out = append(*out, path+"."+k+": missing")
				continue
			}
			diffJSON(path+"."+k, wv, gv, out)
		}
		for k := range g {
			if _, ok := w[k]; !ok {
				*out = append(*out, path+"."+k+": unexpected")
			}
		}
	case []any:
		g, ok := got.([]any)
		if !ok {
			*out = append(*out, path+": want list, got "+py.S(got))
			return
		}
		if len(w) != len(g) {
			*out = append(*out, path+": want "+py.Itoa(len(w))+" items, got "+py.Itoa(len(g)))
			return
		}
		for i := range w {
			diffJSON(path+"["+py.Itoa(i)+"]", w[i], g[i], out)
		}
	case float64:
		g, ok := got.(float64)
		if !ok || math.Abs(py.Round(w, 6)-py.Round(g, 6)) > 1e-9 {
			*out = append(*out, path+": want "+py.Repr(w)+", got "+py.S(got))
		}
	case nil:
		if got != nil {
			*out = append(*out, path+": want null, got "+py.S(got))
		}
	default:
		wb, _ := json.Marshal(want)
		gb, _ := json.Marshal(got)
		if string(wb) != string(gb) {
			*out = append(*out, path+": want "+string(wb)+", got "+string(gb))
		}
	}
}

func TestSharedCases(t *testing.T) {
	paths, _ := filepath.Glob(filepath.Join("..", "..", "..", "tests", "cases", "*.json"))
	if len(paths) == 0 {
		t.Fatal("no cases found")
	}
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		var doc caseDoc
		if err := json.Unmarshal(raw, &doc); err != nil {
			t.Fatalf("%s: %v", path, err)
		}
		market := &store.MarketData{FX: doc.Market.FX, Benchmark: doc.Market.Benchmark, Benchmarks: doc.Market.Benchmarks, Distributions: doc.Market.Distributions, Quotes: doc.Market.Quotes}
		got := ExpectFrom(&doc.Snapshot, market, doc.Today, doc.Filters, doc.Journal)
		var gotJSON any
		b, _ := json.Marshal(got)
		json.Unmarshal(b, &gotJSON)
		var diffs []string
		diffJSON("", doc.Expect, gotJSON, &diffs)
		if len(diffs) > 0 {
			if len(diffs) > 40 {
				diffs = diffs[:40]
			}
			t.Errorf("%s:\n%s", filepath.Base(path), strings.Join(diffs, "\n"))
		}
	}
}
