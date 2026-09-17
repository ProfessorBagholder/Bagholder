package store

import (
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func TestReplacedSourcesAreRefetchedOnce(t *testing.T) {
	s := temp(t)
	s.UpsertPriceHistory("DOT", []DailyBar{{Date: "2026-02-02", Close: 9.5}}, "coingecko")
	s.MarkHistoryFetched("DOT", "2026-02-02", "2026-09-07T00:00:00Z")
	s.UpsertPriceHistory("MAXQ", []DailyBar{{Date: "2026-06-09", Open: py.Ptr(0.4), High: py.Ptr(0.4), Low: py.Ptr(0.4), Close: 0.4, Volume: py.Ptr(1)}}, "cboe_ca")
	s.MarkHistoryFetched("MAXQ", "2025-10-14", "2026-09-07T00:00:00Z")
	s.UpsertPriceHistory("RDDY", []DailyBar{{Date: "2026-02-02", Open: py.Ptr(1), High: py.Ptr(1), Low: py.Ptr(1), Close: 1, Volume: py.Ptr(1)}}, "tmx")
	s.MarkHistoryFetched("RDDY", "2026-02-02", "2026-09-07T00:00:00Z")
	s.UpsertPriceHistory("USDC", []DailyBar{{Date: "2026-02-25", Open: py.Ptr(1), High: py.Ptr(1), Low: py.Ptr(1), Close: 1, Volume: py.Ptr(1)}}, "coinbase")
	s.MarkHistoryFetched("USDC", "2026-01-10", "2026-09-07T00:00:00Z")
	s.UpsertPriceBars("USDC", "1h", []Bar{{Time: 1772000000, Open: py.Ptr(1), High: py.Ptr(1), Low: py.Ptr(1), Close: 1, Volume: py.Ptr(0)}}, "coinbase")
	s.MarkBarsFetched("USDC", "1h", 1768000000, "2026-09-07T00:00:00Z")
	s.UpsertPriceBars("RDDY", "1h", []Bar{{Time: 1768003200, Open: py.Ptr(1), High: py.Ptr(1), Low: py.Ptr(1), Close: 1, Volume: py.Ptr(0)}}, "tmx")
	s.MarkBarsFetched("RDDY", "1h", 1768000000, "2026-09-07T00:00:00Z")
	s.DeleteMeta("history_sources_migrated")
	if err := s.initSchema(); err != nil {
		t.Fatal(err)
	}
	if got := s.PriceHistory("DOT", "", ""); len(got) != 0 {
		t.Errorf("close-only bars are gone: %+v", got)
	}
	if s.HistoryFetch("DOT") != nil {
		t.Error("and their fetch stamp, so the chart refetches")
	}
	if len(s.PriceHistory("MAXQ", "", "")) != 1 {
		t.Error("Cboe's real bars stay")
	}
	if s.HistoryFetch("MAXQ") != nil {
		t.Error("but the span is refetched from TMX, which reaches further back")
	}
	if len(s.PriceHistory("RDDY", "", "")) != 1 {
		t.Error("TMX candles stay")
	}
	if s.HistoryFetch("RDDY") == nil {
		t.Error("the TMX stamp stays")
	}
	if len(s.PriceHistory("USDC", "", "")) != 1 {
		t.Error("real bars stay")
	}
	if s.HistoryFetch("USDC") != nil {
		t.Error("a stamp claiming days its bars do not reach is dropped")
	}
	if s.BarFetchOf("USDC", "1h") != nil {
		t.Error("the same for intraday stamps")
	}
	if s.BarFetchOf("RDDY", "1h") == nil {
		t.Error("an honest intraday stamp stays")
	}
	s.UpsertPriceHistory("DOT", []DailyBar{{Date: "2026-02-03", Close: 9.6}}, "coingecko")
	if err := s.initSchema(); err != nil {
		t.Fatal(err)
	}
	if got := s.PriceHistory("DOT", "", ""); len(got) != 1 {
		t.Errorf("rows added afterwards under an old source name are left alone: %+v", got)
	}
}
