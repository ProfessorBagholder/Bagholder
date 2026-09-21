package store

import "testing"

func watchSymbols(rows []Watch) []string {
	out := make([]string, 0, len(rows))
	for _, w := range rows {
		out = append(out, w.Symbol)
	}
	return out
}

func TestAddListRemove(t *testing.T) {
	s := temp(t)
	before := s.DataVersion()
	row := s.AddWatch("shop", "tsx", "Shopify Inc.", "cad", "", "2026-09-11T14:00:00Z")
	if row == nil || row.Symbol != "SHOP" || row.Exchange != "TSX" || row.Name != "Shopify Inc." || row.Currency != "CAD" || row.AddedAt != "2026-09-11T14:00:00Z" {
		t.Fatalf("%+v", row)
	}
	s.AddWatch("NVDA", "NASDAQ", "", "USD", "", "2026-09-11T14:01:00Z")
	if got := watchSymbols(s.ListWatchlist()); !sameStrings(got, []string{"SHOP", "NVDA"}) {
		t.Errorf("in the order they were added: %v", got)
	}
	again := s.AddWatch("SHOP", "TSX", "", "", "", "2026-09-12T00:00:00Z")
	if again.AddedAt != "2026-09-11T14:00:00Z" || again.Name != "Shopify Inc." {
		t.Errorf("adding a followed listing again keeps its place and its name: %+v", again)
	}
	if got := s.AddWatch("NVDA", "NASDAQ", "NVIDIA Corp", "", "", ""); got.Name != "NVIDIA Corp" {
		t.Errorf("a blank name is filled in: %+v", got)
	}
	if s.DataVersion() == before {
		t.Error("the model's fingerprint follows the list")
	}
	if !s.RemoveWatch("shop", "tsx") {
		t.Error("remove")
	}
	if s.RemoveWatch("SHOP", "TSX") {
		t.Error("removed twice")
	}
	if got := watchSymbols(s.ListWatchlist()); !sameStrings(got, []string{"NVDA"}) {
		t.Errorf("%v", got)
	}
	if got := s.Snapshot(false).Watchlist; len(got) != 1 || got[0].Symbol != "NVDA" {
		t.Errorf("the snapshot carries it to the model: %+v", got)
	}
}
