package model

import (
	"fmt"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func seedRoundTrips(b *testing.B, n int) *store.Store {
	st := store.MustOpen(b.TempDir())
	b.Cleanup(func() { st.Close() })
	day := time.Date(2019, 1, 2, 0, 0, 0, 0, time.UTC)
	rows := make([]store.Activity, 0, n)
	for i := 0; i < n; i++ {
		sym := fmt.Sprintf("S%03d", i%400)
		d := day.AddDate(0, 0, i/8).Format("2006-01-02")
		id := fmt.Sprintf("r%06d", i)
		if (i/400)%2 == 1 {
			rows = append(rows, fixtures.Sell(id, sym, 10, 12.5+float64(i%7), d, fixtures.WithSecurity("sec-"+sym), fixtures.WithSource("wealthsimple")))
		} else {
			rows = append(rows, fixtures.Buy(id, sym, 10, 10+float64(i%5), d, fixtures.WithSecurity("sec-"+sym), fixtures.WithSource("wealthsimple")))
		}
	}
	for i := range rows {
		rows[i].CanonicalID = rows[i].ID
	}
	st.ApplyWealthsimpleMapped(rows)
	return st
}

func BenchmarkViewNewFilterTrades10k(b *testing.B) {
	st := seedRoundTrips(b, 10000)
	m := New(st)
	base := m.Base()
	b.Logf("trades=%d positions=%d cashflow=%d", len(base.Trades), len(base.Positions), len(base.Cashflow))
	var size int
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		out := m.View(map[string]any{"search": fmt.Sprintf("S%03d", i%400)}, "", "")
		size = len(out)
	}
	b.ReportMetric(float64(size), "bytes")
}

func BenchmarkViewNoFilterTrades10k(b *testing.B) {
	st := seedRoundTrips(b, 10000)
	m := New(st)
	m.Base()
	var size int
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		m.viewMu.Lock()
		m.views = nil
		m.viewMu.Unlock()
		out := m.View(nil, "", "")
		size = len(out)
	}
	b.ReportMetric(float64(size), "bytes")
}

func BenchmarkFullRebuildTrades10k(b *testing.B) {
	st := seedRoundTrips(b, 10000)
	m := New(st)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		m.Invalidate(true)
		m.Base()
	}
}
