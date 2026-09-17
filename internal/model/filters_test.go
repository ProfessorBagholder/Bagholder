package model

import (
	"reflect"
	"testing"
)

func TestAccountFilterNarrowsEverything(t *testing.T) {
	base := viewBase()
	v := BuildView(base, lists("account", "Retirement"))
	if v.KPI.Count != 1 {
		t.Errorf("count: %d", v.KPI.Count)
	}
	if v.Trades[0].Symbol != "CCC" {
		t.Errorf("symbol: %q", v.Trades[0].Symbol)
	}
	if len(v.Positions) != 0 {
		t.Errorf("positions: %s", jsonOf(v.Positions))
	}
	if v.Equity.Label != "All accounts" {
		t.Errorf("label: %q", v.Equity.Label)
	}
	v = BuildView(base, lists("account", "Trading"))
	if v.Equity.Label != "Trading" {
		t.Errorf("label: %q", v.Equity.Label)
	}
	if len(v.Positions) != 1 {
		t.Errorf("positions: %s", jsonOf(v.Positions))
	}
}

func TestDateFilters(t *testing.T) {
	base := viewBase()
	v := BuildView(base, map[string]any{"years": anyList("2025")})
	if got := symbolsOf(v.Trades); !reflect.DeepEqual(got, []string{"AAA"}) {
		t.Errorf("years: %v", got)
	}
	v = BuildView(base, map[string]any{"preset": "ytd"})
	if got := symbolSet(v.Trades); !reflect.DeepEqual(got, []string{"BBB", "CCC", "LUNR"}) {
		t.Errorf("ytd: %v", got)
	}
	v = BuildView(base, map[string]any{"from": "2026-02-01", "to": "2026-02-28"})
	if got := symbolsOf(v.Trades); !reflect.DeepEqual(got, []string{"CCC"}) {
		t.Errorf("from/to: %v", got)
	}
	v = BuildView(base, map[string]any{"preset": "1m"})
	if got := symbolsOf(v.Trades); !reflect.DeepEqual(got, []string{"LUNR"}) {
		t.Errorf("1m: %v", got)
	}
}

func TestListAndRangeFilters(t *testing.T) {
	base := viewBase()
	v := BuildView(base, lists("grade", "A"))
	if got := symbolsOf(v.Trades); !reflect.DeepEqual(got, []string{"AAA"}) {
		t.Errorf("grade A: %v", got)
	}
	v = BuildView(base, lists("grade", "Ungraded"))
	if got := symbolSet(v.Trades); !reflect.DeepEqual(got, []string{"CCC", "LUNR"}) {
		t.Errorf("ungraded: %v", got)
	}
	v = BuildView(base, lists("result", "Losers"))
	if got := symbolsOf(v.Trades); !reflect.DeepEqual(got, []string{"BBB"}) {
		t.Errorf("losers: %v", got)
	}
	v = BuildView(base, map[string]any{"ranges": map[string]any{"price": map[string]any{"op": ">", "v": 50}}})
	if got := symbolsOf(v.Trades); !reflect.DeepEqual(got, []string{"BBB"}) {
		t.Errorf("price > 50: %v", got)
	}
	v = BuildView(base, map[string]any{"search": "aa"})
	if got := symbolsOf(v.Trades); !reflect.DeepEqual(got, []string{"AAA"}) {
		t.Errorf("search: %v", got)
	}
	v = BuildView(base, lists("tag", "untagged"))
	if v.KPI.Count != 3 {
		t.Errorf("untagged: %d", v.KPI.Count)
	}
}

func TestFiltersAreCleaned(t *testing.T) {
	f := CleanFilters(map[string]any{
		"lists":  map[string]any{"account": []any{"A", 3, ""}},
		"ranges": map[string]any{"hold": map[string]any{"op": "<", "v": "7"}},
		"preset": "bogus",
		"years":  []any{2025, "abcd"},
		"from":   "2026-1-1",
		"to":     "2026-02-01",
	})
	if !reflect.DeepEqual(f.Lists.Account, []string{"A", "3"}) {
		t.Errorf("account: %v", f.Lists.Account)
	}
	if f.Ranges.Hold.Op != "<" || f.Ranges.Hold.V == nil || *f.Ranges.Hold.V != 7.0 {
		t.Errorf("hold: %s", jsonOf(f.Ranges.Hold))
	}
	if f.Preset != "all" {
		t.Errorf("preset: %q", f.Preset)
	}
	if !reflect.DeepEqual(f.Years, []string{"2025"}) {
		t.Errorf("years: %v", f.Years)
	}
	if f.From != "" {
		t.Errorf("from: %q", f.From)
	}
	if f.To != "2026-02-01" {
		t.Errorf("to: %q", f.To)
	}
}
