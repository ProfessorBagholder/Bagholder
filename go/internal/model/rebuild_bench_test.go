package model

import (
	"fmt"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func seedBook(b *testing.B, n int) *store.Store {
	st := store.MustOpen(b.TempDir())
	b.Cleanup(func() { st.Close() })
	syms := []string{"QNC", "SHOP", "RY", "TD", "ENB", "CNQ", "BNS", "SU", "AEM", "BMO", "NVDA", "AAPL", "MSFT", "TSLA", "AMD", "RKLB"}
	day := time.Date(2019, 1, 2, 0, 0, 0, 0, time.UTC)
	rows := make([]store.Activity, 0, n)
	for i := 0; i < n; i++ {
		sym := syms[i%len(syms)]
		d := day.AddDate(0, 0, i/8).Format("2006-01-02")
		id := fmt.Sprintf("a%06d", i)
		if i%3 == 2 {
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

func BenchmarkFullRebuild10k(b *testing.B) {
	st := seedBook(b, 10000)
	m := New(st)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		m.Invalidate(true)
		m.Base()
	}
}

func BenchmarkBaseOnly10k(b *testing.B) {
	st := seedBook(b, 10000)
	m := New(st)
	m.Base()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		m.Invalidate(false)
		m.Base()
	}
}
