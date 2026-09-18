package model

import (
	"reflect"
	"sort"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func positionsBySymbol(base *Base) map[string]*Position {
	out := map[string]*Position{}
	for _, p := range base.Positions {
		out[p.Symbol] = p
	}
	return out
}

func TestPositionsUseTheQuoteWhenPresent(t *testing.T) {
	kids := fixtures.WithAccount("Kids", "")
	snapshot := snap(buy("b1", "VEQT", 100, 49.76, "2026-01-05", kids), buy("b2", "HBIX", 100, 7.0, "2026-01-05", kids))
	quotes := map[string]store.Quote{"VEQT": {Price: ptr(62.4), PriceChange: ptr(0.08), PercentChange: ptr(0.128), FetchedAt: "2026-09-06T14:00:00Z"}}
	base := BuildBase(snapshot, &store.MarketData{Quotes: quotes}, noJournal(), "2026-09-06", nil)
	p := positionsBySymbol(base)
	if p["VEQT"].Last != 62.4 {
		t.Errorf("VEQT last: %v", p["VEQT"].Last)
	}
	if p["VEQT"].PriceSource != "quote" {
		t.Errorf("VEQT price source: %q", p["VEQT"].PriceSource)
	}
	almost(t, p["VEQT"].Unreal, (62.4-49.76)*100, "VEQT unrealized")
	if deref(p["VEQT"].PriceChange) != 0.08 {
		t.Errorf("VEQT price change: %v", p["VEQT"].PriceChange)
	}
	if p["HBIX"].PriceSource != "fill" {
		t.Errorf("HBIX price source: %q", p["HBIX"].PriceSource)
	}
	if p["HBIX"].Last != 7.0 {
		t.Errorf("HBIX last: %v", p["HBIX"].Last)
	}
	want := []HeldSymbol{{"VEQT", "", "CAD", "Shares"}, {"HBIX", "", "CAD", "Shares"}}
	if got := HeldSymbols(base); !reflect.DeepEqual(got, want) {
		t.Errorf("held symbols: %s", jsonOf(got))
	}
}

func TestPositionsPriceCryptoAndOptionsFromQuotes(t *testing.T) {
	snapshot := snap(
		row(store.Activity{ID: "c1", Category: "trade", ActivityType: "BUY", RawType: "CRYPTO_BUY", Quantity: 0.5, UnitPrice: 100000, NetCashAmount: -50000, TransactionDate: "2026-01-05", Symbol: "BTC", Currency: "CAD", AccountType: "Crypto", SecurityID: "sec-z-btc"}),
		row(store.Activity{ID: "o1", Category: "trade", ActivityType: "BUY", RawType: "OPTIONS_BUY", Quantity: 2, UnitPrice: 0.10, NetCashAmount: -20, TransactionDate: "2026-02-05", Symbol: "QNC 20NOV26 3.00 CALL", Currency: "USD", AccountType: "TFSA", SecurityID: "sec-o-1"}),
	)
	quotes := map[string]store.Quote{"BTC": {Price: ptr(120000.0)}, "QNC 20NOV26 3.00 CALL": {Price: ptr(0.15)}}
	base := BuildBase(snapshot, &store.MarketData{Quotes: quotes}, noJournal(), "2026-09-06", nil)
	by := positionsBySymbol(base)
	btc := by["BTC"]
	if btc.Kind != "Crypto" || btc.PriceSource != "quote" || btc.Last != 120000.0 || btc.MV != 60000.0 {
		t.Errorf("BTC: %s", jsonOf(btc))
	}
	opt := by["QNC 20NOV26 3.00 CALL"]
	if opt.Kind != "Options" || opt.PriceSource != "quote" || opt.Last != 0.15 || opt.MV != 30.0 {
		t.Errorf("option: %s", jsonOf(opt))
	}
	held := [][2]string{}
	for _, h := range HeldSymbols(base) {
		held = append(held, [2]string{h.Symbol, h.Kind})
	}
	sort.Slice(held, func(i, j int) bool { return held[i][0] < held[j][0] })
	if !reflect.DeepEqual(held, [][2]string{{"BTC", "Crypto"}, {"QNC 20NOV26 3.00 CALL", "Options"}}) {
		t.Errorf("held: %v", held)
	}
}

func TestACoinsPriceNeverPricesAShareWithTheSameSymbol(t *testing.T) {
	snapshot := snap(
		row(store.Activity{ID: "c1", Category: "trade", ActivityType: "BUY", RawType: "CRYPTO_BUY", Quantity: 0.5, UnitPrice: 100000, NetCashAmount: -50000, TransactionDate: "2026-01-05", Symbol: "BTC", Currency: "CAD", AccountType: "Crypto", SecurityID: "sec-z-btc-1"}),
		row(store.Activity{ID: "s1", Category: "trade", ActivityType: "BUY", RawType: "DIY_BUY", Quantity: 4653, UnitPrice: 1.75, NetCashAmount: -8142.75, TransactionDate: "2026-02-05", Symbol: "BTC", Currency: "CAD", AccountType: "TFSA", SecurityID: "sec-s-btc-warrant"}),
	)
	quotes := map[string]store.Quote{"BTC": {Price: ptr(109998.0), Source: "coinbase"}}
	base := BuildBase(snapshot, &store.MarketData{Quotes: quotes}, noJournal(), "2026-09-06", nil)
	by := map[[2]string]*Position{}
	for _, p := range base.Positions {
		by[[2]string{p.Symbol, p.Kind}] = p
	}
	coin := by[[2]string{"BTC", "Crypto"}]
	if coin == nil || coin.PriceSource != "quote" || coin.Last != 109998.0 {
		t.Errorf("coin: %s", jsonOf(coin))
	}
	share := by[[2]string{"BTC", "Shares"}]
	if share == nil || share.PriceSource != "fill" || share.Last != 1.75 || py.Round(share.MV, 2) != 8142.75 {
		t.Errorf("the share keeps its fill price rather than the coin's: %s", jsonOf(share))
	}
	if !QuoteFits(&store.Quote{Price: ptr(1.0)}, "Shares") {
		t.Error("a quote with no source stated is the kind's own")
	}
	if QuoteFits(&store.Quote{Price: ptr(1.0), Source: "tmx"}, "Crypto") {
		t.Error("a TMX quote prices a coin")
	}
}

func TestPortfolioTilesSumWealthsimpleFiguresOverTheAccountsInScope(t *testing.T) {
	acts := []store.Activity{
		buy("b1", "AAA", 10, 10, "2026-01-05"),
		buy("b2", "BBB", 5, 20, "2026-01-06", fixtures.WithAccount("Kids", "acct-2"), fixtures.WithCurrency("USD")),
	}
	snapshot := &store.Snapshot{
		Activities: acts,
		Accounts: []store.Account{
			{ID: "acct-1", Nickname: "Trading", Currency: "CAD", NetLiquidationValue: ptr(1500.0), UnifiedAccountType: "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
			{ID: "acct-2", Nickname: "Kids", Currency: "CAD", NetLiquidationValue: ptr(400.0), UnifiedAccountType: "SELF_DIRECTED_JOINT_NON_REGISTERED_MARGIN"},
			{ID: "acct-3", Nickname: "Cash", Currency: "CAD", NetLiquidationValue: ptr(25.0), UnifiedAccountType: "CASH"},
			{ID: "acct-4", Nickname: "Old", Currency: "CAD", NetLiquidationValue: ptr(999.0), Status: "closed", UnifiedAccountType: "SELF_DIRECTED_NON_REGISTERED_MARGIN"},
			{ID: "acct-5", Nickname: "TFSA", Currency: "CAD", NetLiquidationValue: ptr(0.0), UnifiedAccountType: "SELF_DIRECTED_TFSA"},
		},
		Balances: []store.Balance{
			{AccountID: "acct-1", SecurityID: "sec-c-cad", Quantity: ptr(-300.0)},
			{AccountID: "acct-1", SecurityID: "sec-c-usd", Quantity: ptr(-10.0)},
			{AccountID: "acct-2", SecurityID: "sec-c-cad", Quantity: ptr(50.0)},
			{AccountID: "acct-1", SecurityID: "sec-s-aaa", Quantity: ptr(10.0)},
			{AccountID: "acct-4", SecurityID: "sec-c-cad", Quantity: ptr(-5000.0)},
		},
		Securities: []store.Security{
			{ID: "sec-c-cad", Symbol: "CAD", Currency: "CAD"},
			{ID: "sec-c-usd", Symbol: "USD", Currency: "USD"},
			{ID: "sec-s-aaa", Symbol: "AAA", Currency: "CAD"},
		},
		Margin: []store.Margin{
			{AccountID: "acct-1", BuyingPower: ptr(700.0), Currency: "CAD", Unavailable: ""},
			{AccountID: "acct-2", BuyingPower: nil, Currency: "CAD", Unavailable: "UnavailableSecurities (1 securities)"},
			{AccountID: "acct-5", BuyingPower: ptr(5638.24), Currency: "CAD", Unavailable: ""},
		},
	}
	market := &store.MarketData{FX: map[string]float64{"2026-02-01": 1.5}, Quotes: map[string]store.Quote{"AAA": {Price: ptr(12.0), PriceChange: ptr(0.5), PercentChange: ptr(4.35)}, "BBB": {Price: ptr(30.0)}}}
	base := BuildBase(snapshot, market, map[string]store.JournalEntry{"rt:b1": {Grade: "B", Thesis: "hold", Tags: []string{"core"}}}, "2026-02-01", nil)
	v := BuildView(base, map[string]any{})
	pf := v.Portfolio
	almost(t, pf.MarketValue, 120+150*1.5, "market value")
	almost(t, pf.CostBasis, 100+100*1.5, "cost basis")
	almost(t, pf.Unrealized, 20+50*1.5, "unrealized")
	if pf.PositionCount != 2 || pf.AccountCount != 2 {
		t.Errorf("counts: %d positions, %d accounts", pf.PositionCount, pf.AccountCount)
	}
	almost(t, deref(pf.Nav), 1500+400+25, "every open account counts, cash accounts included, closed ones not")
	almost(t, pf.MarginUsed, 300+10*1.5, "negative cash per currency, in CAD")
	if !reflect.DeepEqual(pf.MarginUsedBy, map[string]float64{"CAD": 300.0, "USD": 10.0}) {
		t.Errorf("the closed account's cash is not margin used: %v", pf.MarginUsedBy)
	}
	almost(t, deref(pf.AvailableMargin), 700.0, "the TFSA's buying power is not counted")
	if !reflect.DeepEqual(pf.AvailableMarginUnavailable, []string{"Kids"}) {
		t.Errorf("unavailable: %v", pf.AvailableMarginUnavailable)
	}
	var aaa, bbb *Position
	for _, p := range v.Positions {
		switch p.Symbol {
		case "AAA":
			aaa = p
		case "BBB":
			bbb = p
		}
	}
	almost(t, deref(aaa.DayChange), 10*0.5, "AAA day change")
	if aaa.Grade != "B" {
		t.Errorf("AAA grade: %q", aaa.Grade)
	}
	sides := []string{}
	for _, f := range aaa.Fills {
		sides = append(sides, f.Side)
	}
	if !reflect.DeepEqual(sides, []string{"BUY"}) {
		t.Errorf("AAA fill sides: %v", sides)
	}
	if bbb.DayChange != nil {
		t.Errorf("no change on the quote, no day change: %v", *bbb.DayChange)
	}
	kids := BuildView(base, lists("account", "Kids")).Portfolio
	almost(t, deref(kids.Nav), 400.0, "Kids nav")
	almost(t, kids.MarginUsed, 0.0, "Kids margin used")
	if kids.AvailableMargin != nil {
		t.Errorf("Kids available margin: %v", *kids.AvailableMargin)
	}
	if !reflect.DeepEqual(kids.AvailableMarginUnavailable, []string{"Kids"}) {
		t.Errorf("Kids unavailable: %v", kids.AvailableMarginUnavailable)
	}
	almost(t, kids.MarketValue, 150*1.5, "Kids market value")
	empty := BuildView(BuildBase(&store.Snapshot{Activities: acts}, market, noJournal(), "2026-02-01", nil), map[string]any{}).Portfolio
	if empty.Nav != nil {
		t.Errorf("nav without accounts: %v", *empty.Nav)
	}
	if empty.AvailableMargin != nil {
		t.Errorf("available margin without accounts: %v", *empty.AvailableMargin)
	}
	if empty.MarginUsed != 0.0 {
		t.Errorf("margin used without accounts: %v", empty.MarginUsed)
	}
}

func TestIntradayArchiveCoversRecentTradesAndHoldings(t *testing.T) {
	tfsa := fixtures.WithAccount("TFSA", "")
	acts := []store.Activity{
		buy("b1", "OLD", 10, 5, "2024-01-05", tfsa), sell("s1", "OLD", 10, 6, "2024-02-05", tfsa),
		buy("b2", "NEW", 10, 5, "2026-03-01", tfsa), sell("s2", "NEW", 10, 6, "2026-04-01", tfsa),
		buy("b3", "NEW", 10, 5, "2026-06-01", tfsa), sell("s3", "NEW", 10, 6, "2026-07-01", tfsa),
		buy("b4", "HELD", 10, 5, "2025-05-01", tfsa),
	}
	base := BuildBase(snap(acts...), mkt(nil, nil), noJournal(), "2026-09-07", nil)
	by := map[string]ArchiveSymbol{}
	for _, r := range IntradayArchiveSymbols(base, "") {
		by[r.Symbol] = r
	}
	if _, ok := by["OLD"]; ok {
		t.Error("closed long before the window")
	}
	if by["NEW"].Start != "2026-03-01" {
		t.Errorf("earliest entry within the window: %q", by["NEW"].Start)
	}
	if by["HELD"].Start != "2025-09-07" {
		t.Errorf("an old holding is wanted from the window start: %q", by["HELD"].Start)
	}
	optActs := append(append([]store.Activity{}, acts...),
		row(store.Activity{ID: "o1", Category: "trade", ActivityType: "BUY", RawType: "OPTIONS_BUY", Quantity: 2, UnitPrice: 0.10, NetCashAmount: -20, TransactionDate: "2026-05-05", Symbol: "LUNR 15JAN27 10.00 CALL", Currency: "USD", AccountType: "TFSA", SecurityID: "sec-o-1"}),
	)
	base = BuildBase(snap(optActs...), mkt(nil, nil), noJournal(), "2026-09-07", nil)
	by = map[string]ArchiveSymbol{}
	for _, r := range IntradayArchiveSymbols(base, "") {
		by[r.Symbol] = r
	}
	lunr, ok := by["LUNR"]
	if !ok {
		t.Fatal("an option position is archived as its underlying")
	}
	if lunr.Kind != "Shares" || lunr.Start != "2026-05-05" {
		t.Errorf("LUNR: %+v", lunr)
	}
}
