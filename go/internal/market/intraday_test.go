package market

import (
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func TestIntradayBarsAreCachedPerTimeframe(t *testing.T) {
	now := utc(2025, 12, 1, 12, 0, 0)
	c, s := newTestClient(t, now)
	r := rec("TSLA", "NASDAQ", "USD", "Shares")
	t0 := utc(2025, 11, 10, 14, 30, 0).Unix()
	minutes := tmxIntraday(minuteRow("2025-11-10", "09:30", "-05:00", 1, 2, 0.5, 1.5, 3), minuteRow("2025-11-10", "10:30", "-05:00", 1.5, 2, 1, 1.8, 4))
	s.set(func(call stubCall) (int, string, error) {
		p, ok := parseTMX(call.Body)
		if !ok || p.Op != "getCompanyChart" {
			return 0, "", errNoRoute
		}
		if p.Var("from") == "2025-11-05" {
			return 200, minutes, nil
		}
		return 200, tmxIntraday(), nil
	})
	h := c.EnsureIntraday(r, "1h", "2025-11-05", "2025-11-20", now, 1, true)
	eq(t, closes(h), []float64{1.5, 1.8}, "")
	eq(t, fv(h[0].Open), 1.0, "")
	eq(t, h[0].Time, t0, "")
	s.reset()
	four := c.EnsureIntraday(r, "4h", "2025-11-05", "2025-11-20", now, 1, true)
	eq(t, len(s.tmxPosts()), 0, "one minute fetch fills both timeframes")
	eq(t, closes(four), []float64{1.8}, "")
}

func TestArchiveSweepIsPacedAndTopsUpIncrementally(t *testing.T) {
	now := utc(2026, 9, 7, 12, 0, 0)
	c, s := newTestClient(t, now)
	recs := []Rec{}
	for _, sym := range []string{"AAA", "BBB", "CCC"} {
		recs = append(recs, Rec{Symbol: sym, Exchange: "TSX", Currency: "CAD", Kind: "Shares", Start: "2026-08-01"})
	}
	recs = append(recs, Rec{Symbol: "QNC 20NOV26 3.00 CALL", Exchange: "NYSE", Currency: "USD", Kind: "Options", Start: "2026-08-01"})
	t0 := utc(2026, 8, 3, 13, 30, 0).Unix()
	s.set(func(call stubCall) (int, string, error) {
		p, ok := parseTMX(call.Body)
		if !ok || p.Op != "getCompanyChart" {
			return 0, "", errNoRoute
		}
		if p.Var("from") <= "2026-08-03" && "2026-08-03" <= p.Var("to") {
			return 200, tmxIntraday(minuteRow("2026-08-03", "09:30", "-04:00", 1, 1, 1, 1, 1)), nil
		}
		return 200, tmxIntraday(), nil
	})
	fetched := func() []string {
		out := []string{}
		for _, sym := range s.tmxSymbols("getCompanyChart") {
			if !in(out, sym) {
				out = append(out, sym)
			}
		}
		return out
	}
	eq(t, c.ArchiveIntraday(recs, now, 2), []string{"AAA", "BBB"}, "")
	eq(t, len(fetched()), 2, "")
	eq(t, c.ArchiveIntraday(recs, now, 2), []string{"CCC"}, "never-fetched first, options skipped")
	eq(t, c.ArchiveIntraday(recs, now.Add(2*time.Hour), 2), []string{}, "fresh copies are left alone")
	s.reset()
	eq(t, c.ArchiveIntraday(recs, now.Add(25*time.Hour), 8), []string{"AAA", "BBB", "CCC"}, "")
	from := ""
	for _, p := range s.tmxPosts() {
		if p.Op == "getCompanyChart" && p.Sym() == "AAA" && (from == "" || p.Var("from") < from) {
			from = p.Var("from")
		}
	}
	eq(t, from, py.DateStr(time.Unix(t0-2*86400, 0).UTC()), "")
	eq(t, c.Store.BarFetchOf("AAA", "1h").StartTs, utc(2026, 8, 1, 0, 0, 0).Unix(), "the archived span still starts where it began")
}

func TestDailyArchiveKeepsBarsOnlyForSourcesThatForgetThem(t *testing.T) {
	now := utc(2026, 9, 7, 12, 0, 0)
	c, s := newTestClient(t, now)
	recs := []Rec{
		{Symbol: "HBIX", Exchange: "Cboe Canada", Currency: "CAD", Kind: "Shares", Start: "2026-06-01"},
		{Symbol: "BTC", Exchange: "Crypto", Currency: "CAD", Kind: "Crypto", Start: "2026-06-01"},
		{Symbol: "RDDY", Exchange: "TSX", Currency: "CAD", Kind: "Shares", Start: "2026-06-01"},
	}
	s.set(func(call stubCall) (int, string, error) {
		if call.URL == TMXURL {
			return 200, tmxSeries(tmxRow("2026-06-02", 1, 1, 1, 1, 1)), nil
		}
		return 0, "", errNoRoute
	})
	eq(t, c.ArchiveDaily(recs, now, ArchiveBatch), []string{}, "every history source keeps full history itself; nothing to archive daily")
	eq(t, s.count(), 0, "")
}

func TestBackgroundSweepNeverAsksYahoo(t *testing.T) {
	now := utc(2026, 9, 7, 12, 0, 0)
	c, s := newTestClient(t, now)
	old := Rec{Symbol: "TSLA", Exchange: "NASDAQ", Currency: "USD", Kind: "Shares", Start: "2025-01-15"}
	eq(t, c.ArchiveIntraday([]Rec{old}, now, ArchiveBatch), []string{"TSLA"}, "")
	eq(t, s.urls("yahoo"), []string{}, "the sweep leaves the rate-limited source alone")
	c.EnsureIntraday(old, "1h", "2025-01-15", "2025-02-01", now, 1, true)
	eq(t, len(s.urls("yahoo")), 1, "a chart someone opens does ask it")
}

func TestATimeframeAFetchCouldNotSupplyIsNotAskedForAgainForAWhile(t *testing.T) {
	now := utc(2026, 9, 7, 12, 0, 0)
	c, s := newTestClient(t, now)
	r := rec("TSLA", "NASDAQ", "USD", "Shares")
	eq(t, c.IntradayReady(r, "1h", "2025-01-15", now), false, "")
	s.set(func(call stubCall) (int, string, error) {
		if call.URL == TMXURL {
			return 0, "", errNoRoute
		}
		return 200, jsonText(obj("chart", obj("result", []any{}))), nil
	})
	eq(t, len(c.EnsureIntraday(r, "1h", "2025-01-15", "2025-02-01", now, 1, true)), 0, "")
	eq(t, len(s.urls("yahoo")), 1, "")
	eq(t, c.IntradayReady(r, "1h", "2025-01-15", now), true, "nothing to wait for after a miss")
	eq(t, c.OfferedTimeframes(r, "2025-01-15", now), []string{"1d", "1w", "1M"}, "the chart falls back to daily instead of an empty hourly view")
	later := now.Add((IntradayRetryMinutes + 1) * time.Minute)
	eq(t, c.IntradayReady(r, "1h", "2025-01-15", later), false, "tried again after the retry window")
	eq(t, c.OfferedTimeframes(r, "2025-01-15", later), []string{"1h", "4h", "1d", "1w", "1M"}, "")
}
