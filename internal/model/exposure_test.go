package model

import (
	"math"
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

type sliceView struct {
	Name  string
	Value float64
}

func sliceViews(rows []ExposureSlice) []sliceView {
	out := []sliceView{}
	for _, r := range rows {
		out = append(out, sliceView{r.Name, math.Round(r.Value*100) / 100})
	}
	return out
}

func fptr(v float64) *float64 { return &v }

func usdDoubles(v float64, c string) float64 {
	if c == "USD" {
		return v * 2.0
	}
	return v
}

func TestPositionsSpreadByTheirRecords(t *testing.T) {
	positions := []*Position{
		{PositionCore: PositionCore{MV: 1000.0, Currency: "CAD", SecurityID: "a", Short: false, Kind: "Shares"}},
		{PositionCore: PositionCore{MV: 500.0, Currency: "USD", SecurityID: "b", Short: false, Kind: "Shares"}},
		{PositionCore: PositionCore{MV: 300.0, Currency: "CAD", SecurityID: "c", Short: false, Kind: "Shares"}},
		{PositionCore: PositionCore{MV: 100.0, Currency: "CAD", SecurityID: "a", Short: true, Kind: "Shares"}},
		{PositionCore: PositionCore{MV: 200.0, Currency: "CAD", SecurityID: "btc", Short: false, Kind: "Crypto"}},
		{PositionCore: PositionCore{MV: 50.0, Currency: "USD", SecurityID: "opt", Short: true, Kind: "Options", Underlying: "AAPL"}},
	}
	exposures := map[string]store.Exposure{
		"a":              {Sectors: map[string]float64{"Financials": 1.0}, Countries: map[string]float64{"Canada": 1.0}, Coverage: 1.0},
		"b":              {Sectors: map[string]float64{"Information Technology": 0.5, "Energy": 0.25}, Countries: map[string]float64{"United States": 0.75}, Coverage: 0.75},
		"share:AAPL::US": {Sectors: map[string]float64{"Information Technology": 1.0}, Countries: map[string]float64{"United States": 1.0}, Coverage: 1.0},
	}
	sectors, regions := exposureSlices(positions, exposures, usdDoubles)
	wantSectors := []sliceView{{"Financials", 1100.0}, {"Information Technology", 600.0}, {"Energy", 250.0}, {"Digital assets", 200.0}, {"Not classified", 550.0}}
	if got := sliceViews(sectors); !reflect.DeepEqual(got, wantSectors) {
		t.Errorf("the same positions as Allocation, the short included; the contract counts as its underlying; b's uncovered quarter and c, which has no record, are unclassified; the coin is Digital assets: got %v, want %v", got, wantSectors)
	}
	sum := 0.0
	for _, s := range sectors {
		sum += s.Share
	}
	if math.Abs(sum-1.0) >= 5e-8 {
		t.Errorf("shares sum to one: got %v", sum)
	}
	wantRegions := []sliceView{{"Canada", 1100.0}, {"United States", 850.0}, {"Not classified", 750.0}}
	if got := sliceViews(regions); !reflect.DeepEqual(got, wantRegions) {
		t.Errorf("a coin has no country: got %v, want %v", got, wantRegions)
	}
}

func TestAStoredAliasFoldsWhenRead(t *testing.T) {
	positions := []*Position{
		{PositionCore: PositionCore{MV: 100.0, Currency: "CAD", SecurityID: "a", Short: false, Kind: "Shares"}},
		{PositionCore: PositionCore{MV: 100.0, Currency: "CAD", SecurityID: "b", Short: false, Kind: "Shares"}},
	}
	exposures := map[string]store.Exposure{
		"a": {Sectors: map[string]float64{"Communication": 1.0}, Countries: map[string]float64{}, Coverage: 1.0},
		"b": {Sectors: map[string]float64{"Communication Services": 1.0}, Countries: map[string]float64{}, Coverage: 1.0},
	}
	sectors, _ := exposureSlices(positions, exposures, func(v float64, c string) float64 { return v })
	want := []sliceView{{"Communication Services", 200.0}}
	if got := sliceViews(sectors); !reflect.DeepEqual(got, want) {
		t.Errorf("got %v, want %v", got, want)
	}
}

type tileView struct {
	Symbol        string
	Value         float64
	Sector        string
	PercentChange *float64
}

func TestHeatmapTilesTakeTheDominantSector(t *testing.T) {
	positions := []*Position{
		{PositionCore: PositionCore{ID: "p1", MV: 1000.0, Currency: "CAD", SecurityID: "a", Short: false, Kind: "Shares", Symbol: "XEQT", Exchange: "TSX", PercentChange: fptr(0.4)}},
		{PositionCore: PositionCore{ID: "p2", MV: 500.0, Currency: "USD", SecurityID: "b", Short: false, Kind: "Shares", Symbol: "NVDA", Exchange: "NASDAQ", PercentChange: fptr(-1.2)}},
		{PositionCore: PositionCore{ID: "p3", MV: 200.0, Currency: "CAD", SecurityID: "btc", Short: false, Kind: "Crypto", Symbol: "BTC", Exchange: "Crypto", PercentChange: nil}},
		{PositionCore: PositionCore{ID: "p4", MV: 50.0, Currency: "USD", SecurityID: "opt", Short: false, Kind: "Options", Symbol: "AAPL 20DEC26 200.00 CALL", Underlying: "AAPL", Exchange: "", PercentChange: fptr(3.0)}},
		{PositionCore: PositionCore{ID: "p5", MV: 0.0, Currency: "CAD", SecurityID: "z", Short: false, Kind: "Shares", Symbol: "ZERO", Exchange: "TSX", PercentChange: nil}},
		{PositionCore: PositionCore{ID: "p6", MV: 250.0, Currency: "USD", SecurityID: "b", Short: false, Kind: "Shares", Symbol: "NVDA", Exchange: "NASDAQ", PercentChange: fptr(-1.2)}},
	}
	exposures := map[string]store.Exposure{
		"a":              {Sectors: map[string]float64{"Financials": 0.3, "Information Technology": 0.45, "Energy": 0.25}},
		"b":              {Sectors: map[string]float64{"Information Technology": 1.0}},
		"share:AAPL::US": {Sectors: map[string]float64{"Information Technology": 1.0}},
	}
	tiles := heatmapItems(positions, exposures, usdDoubles)
	got := []tileView{}
	for _, x := range tiles {
		got = append(got, tileView{x.Symbol, x.Value, x.Sector, x.PercentChange})
	}
	want := []tileView{{"XEQT", 1000.0, "Information Technology", fptr(0.4)}, {"NVDA", 1500.0, "Information Technology", fptr(-1.2)}, {"BTC", 200.0, "Digital assets", nil}, {"AAPL 20DEC26 200.00 CALL", 100.0, "Information Technology", fptr(3.0)}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("a fund sits under the sector it weights most, a coin under Digital assets, a contract under its underlying; nothing worth nothing; a symbol held in two accounts is one tile: got %v, want %v", got, want)
	}
}

type watchView struct {
	Symbol        string
	Last          *float64
	PercentChange *float64
	Sector        string
	PositionID    *string
}

func TestWatchRowsCarryTheQuoteTheSectorAndTheHolding(t *testing.T) {
	base := &Base{
		Quotes:    map[string]store.Quote{"SHOP@TSX": {Price: fptr(212.06), PriceChange: fptr(1.56), PercentChange: fptr(0.74)}},
		Exposures: map[string]store.Exposure{"share:SHOP:": {Sectors: map[string]float64{"Information Technology": 1.0}}},
		Watchlist: []store.Watch{{Symbol: "SHOP", Exchange: "TSX", Name: "Shopify Inc.", Currency: "CAD"}, {Symbol: "RKLB", Exchange: "NASDAQ", Name: "Rocket Lab", Currency: "USD"}},
	}
	positions := []*Position{{PositionCore: PositionCore{ID: "p9", Symbol: "SHOP", Exchange: "TSX"}}}
	rows := watchRows(base, positions)
	got := []watchView{}
	for _, r := range rows {
		got = append(got, watchView{r.Symbol, r.Last, r.PercentChange, r.Sector, r.PositionID})
	}
	p9 := "p9"
	want := []watchView{{"SHOP", fptr(212.06), fptr(0.74), "Information Technology", &p9}, {"RKLB", nil, nil, "Not classified", nil}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("a quote and a record when the app has them, dashes otherwise; the held one names its holding: got %v, want %v", got, want)
	}
}
