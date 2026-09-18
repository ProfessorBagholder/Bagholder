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

type replayInputs struct {
	Snapshot store.Snapshot                `json:"snapshot"`
	Market   caseMarket                    `json:"market"`
	Journal  map[string]store.JournalEntry `json:"journal"`
	Today    string                        `json:"today"`
}

type replayView struct {
	Inputs  replayInputs `json:"inputs"`
	RawSnap struct {
		Tiles json.RawMessage `json:"tiles"`
	} `json:"-"`
	Filters any            `json:"filters"`
	View    map[string]any `json:"view"`
}

func closeEnough(w, g float64) bool {
	if w == g {
		return true
	}
	tol := 1e-9 * math.Max(1, math.Abs(w))
	return math.Abs(w-g) <= tol
}

func diffLoose(path string, want, got any, out *[]string) {
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
			diffLoose(path+"."+k, wv, gv, out)
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
			diffLoose(path+"["+py.Itoa(i)+"]", w[i], g[i], out)
		}
	case float64:
		g, ok := got.(float64)
		if !ok || !closeEnough(w, g) {
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

func roundTrip(v any) any {
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	var out any
	json.Unmarshal(b, &out)
	return out
}

func replayDir(t *testing.T) string {
	dir := os.Getenv("BAGHOLDER_REPLAY_DIR")
	if dir == "" {
		t.Skip("BAGHOLDER_REPLAY_DIR not set")
	}
	return dir
}

func TestReplayViews(t *testing.T) {
	dir := replayDir(t)
	paths, _ := filepath.Glob(filepath.Join(dir, "view-*.json"))
	sort.Strings(paths)
	failed := 0
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		var doc replayView
		if err := json.Unmarshal(raw, &doc); err != nil {
			t.Fatalf("%s: %v", path, err)
		}
		var probe struct {
			Inputs struct {
				Snapshot struct {
					Tiles json.RawMessage `json:"tiles"`
				} `json:"snapshot"`
			} `json:"inputs"`
		}
		json.Unmarshal(raw, &probe)
		doc.Inputs.Snapshot.TilesSaved = len(probe.Inputs.Snapshot.Tiles) > 0 && string(probe.Inputs.Snapshot.Tiles) != "null"
		mk := doc.Inputs.Market
		market := &store.MarketData{FX: mk.FX, Benchmark: mk.Benchmark, Benchmarks: mk.Benchmarks, Distributions: mk.Distributions, Quotes: mk.Quotes}
		journal := doc.Inputs.Journal
		if journal == nil {
			journal = map[string]store.JournalEntry{}
		}
		base := BuildBase(&doc.Inputs.Snapshot, market, journal, doc.Inputs.Today, nil)
		view := BuildView(base, doc.Filters)
		for _, tr := range view.Trades {
			tr.Detail = true
		}
		for _, p := range view.Positions {
			p.Detail = true
		}
		got := roundTrip(view).(map[string]any)
		delete(got, "generated")
		delete(doc.View, "generated")
		var diffs []string
		diffLoose("", doc.View, got, &diffs)
		if len(diffs) > 0 {
			failed++
			if len(diffs) > 30 {
				diffs = diffs[:30]
			}
			t.Errorf("%s:\n%s", filepath.Base(path), strings.Join(diffs, "\n"))
		}
	}
	t.Logf("%d views replayed, %d differ", len(paths), failed)
}

type replayFifo struct {
	Activities []store.Activity `json:"activities"`
	Result     map[string]any   `json:"result"`
}

func TestReplayFifo(t *testing.T) {
	dir := replayDir(t)
	paths, _ := filepath.Glob(filepath.Join(dir, "fifo-*.json"))
	sort.Strings(paths)
	failed := 0
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		var doc replayFifo
		if err := json.Unmarshal(raw, &doc); err != nil {
			t.Fatalf("%s: %v", path, err)
		}
		var probe struct {
			Activities []map[string]json.RawMessage `json:"activities"`
		}
		json.Unmarshal(raw, &probe)
		acts := make([]*Act, 0, len(doc.Activities))
		for i := range doc.Activities {
			a := &doc.Activities[i]
			if _, ok := probe.Activities[i]["flags"]; ok {
				a.Normalized = true
				if a.Flags == nil {
					a.Flags = []string{}
				}
			}
			acts = append(acts, a)
		}
		res := MatchFIFO(acts)
		got := roundTrip(map[string]any{"closed": res.Closed, "open": res.Open, "unmatched": res.Unmatched})
		var diffs []string
		diffLoose("", doc.Result, got, &diffs)
		if len(diffs) > 0 {
			failed++
			if len(diffs) > 30 {
				diffs = diffs[:30]
			}
			t.Errorf("%s:\n%s", filepath.Base(path), strings.Join(diffs, "\n"))
		}
	}
	t.Logf("%d fifo runs replayed, %d differ", len(paths), failed)
}
