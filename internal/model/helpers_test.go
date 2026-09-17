package model

import (
	"encoding/json"
	"math"
	"sort"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var row = fixtures.Act
var buy = fixtures.Buy
var sell = fixtures.Sell

func bare(a store.Activity) store.Activity {
	a = fixtures.Act(a)
	a.Symbol = ""
	return a
}

func ptrs(acts []store.Activity) []*Act {
	out := make([]*Act, 0, len(acts))
	for i := range acts {
		out = append(out, &acts[i])
	}
	return out
}

func tol(places int) float64 { return 0.5 * math.Pow(10, -float64(places)) }

func almost(t *testing.T, got, want float64, msg string) {
	t.Helper()
	almostPlaces(t, got, want, 7, msg)
}

func almostPlaces(t *testing.T, got, want float64, places int, msg string) {
	t.Helper()
	if math.IsNaN(got) || math.Abs(got-want) >= tol(places) {
		t.Errorf("%s: got %v, want %v", msg, got, want)
	}
}

func almostDelta(t *testing.T, got, want, delta float64, msg string) {
	t.Helper()
	if math.IsNaN(got) || math.Abs(got-want) > delta {
		t.Errorf("%s: got %v, want %v (delta %v)", msg, got, want, delta)
	}
}

func snap(acts ...store.Activity) *store.Snapshot {
	return &store.Snapshot{Activities: acts}
}

func mkt(fx, bench map[string]float64) *store.MarketData {
	return &store.MarketData{FX: fx, Benchmark: bench}
}

func noJournal() map[string]store.JournalEntry { return map[string]store.JournalEntry{} }

func anyList(vals ...string) []any {
	out := make([]any, 0, len(vals))
	for _, v := range vals {
		out = append(out, v)
	}
	return out
}

func lists(key string, vals ...string) map[string]any {
	return map[string]any{"lists": map[string]any{key: anyList(vals...)}}
}

func symbolsOf(trades []*Trade) []string {
	out := []string{}
	for _, t := range trades {
		out = append(out, t.Symbol)
	}
	return out
}

func symbolSet(trades []*Trade) []string {
	seen := map[string]bool{}
	for _, t := range trades {
		seen[t.Symbol] = true
	}
	return sortedSet(seen)
}

func jsonOf(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return string(b)
}

func sameJSON(a, b any) bool { return jsonOf(a) == jsonOf(b) }

func hasFlag(flags []string, f string) bool {
	for _, x := range flags {
		if x == f {
			return true
		}
	}
	return false
}

func sumPnl(closed []*Slice) float64 {
	s := 0.0
	for _, c := range closed {
		s += c.Pnl
	}
	return s
}

func deref(p *float64) float64 {
	if p == nil {
		return math.NaN()
	}
	return *p
}

func ptr(v float64) *float64 { return &v }

func sortedStrings(in []string) []string {
	out := append([]string{}, in...)
	sort.Strings(out)
	return out
}

func tempStore(t *testing.T) *store.Store {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	t.Cleanup(func() { st.Close() })
	return st
}

func viewJSON(t *testing.T, m *Model, filters any, detail string) map[string]any {
	t.Helper()
	var out map[string]any
	if err := json.Unmarshal(m.View(filters, detail), &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func listAt(m map[string]any, key string) []any {
	v, _ := m[key].([]any)
	return v
}

func mapAt(m map[string]any, key string) map[string]any {
	v, _ := m[key].(map[string]any)
	return v
}

func tileByLabel(tiles []map[string]any) map[string]map[string]any {
	out := map[string]map[string]any{}
	for _, tile := range tiles {
		out[tile["label"].(string)] = tile
	}
	return out
}

func tileLabels(tiles []map[string]any) []string {
	out := []string{}
	for _, tile := range tiles {
		out = append(out, tile["label"].(string))
	}
	return out
}
