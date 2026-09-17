package model

import (
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func TestATileCarriesTheRateBesideThePublishedPriceAndTheDayRunsTheOtherWay(t *testing.T) {
	base := &Base{
		Tiles:      []store.Tile{{Symbol: "ZQ", Exchange: "CBOT"}, {Symbol: "ES", Exchange: "CME"}},
		TilesSaved: true,
		Quotes: map[string]store.Quote{
			WatchQuoteKey("ZQ", "CBOT"): {Price: py.Ptr(96.13), PriceChange: py.Ptr(-0.157), PercentChange: py.Ptr(-0.163)},
			WatchQuoteKey("ES", "CME"):  {Price: py.Ptr(7674.0), PriceChange: py.Ptr(18.0), PercentChange: py.Ptr(0.24)},
		},
	}
	rows := map[string]TileRow{}
	for _, r := range tileRows(base) {
		rows[r.Symbol] = r
	}
	zq := rows["ZQ"]
	if zq.Last == nil || *zq.Last != 96.13 || zq.Rate == nil || *zq.Rate != 3.87 || zq.RateChange == nil || *zq.RateChange != 0.157 {
		t.Errorf("the price as published, the rate it prices, and a day that cut the price raised the rate: last %v rate %v rateChange %v", py.Deref(zq.Last, -1), py.Deref(zq.Rate, -1), py.Deref(zq.RateChange, -1))
	}
	if es := rows["ES"]; es.Rate != nil {
		t.Errorf("nothing else carries one: ES rate %v", *es.Rate)
	}
}
