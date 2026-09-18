package model

import (
	"math"
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func TestMonthlyDistributionsRunToTheCurrentMonth(t *testing.T) {
	cashflow := fixtures.WithAccount("Cashflow", "")
	snapshot := snap(
		buy("b1", "RDDY", 100, 5, "2026-05-01", cashflow),
		fixtures.Dividend("d1", "RDDY", 100, 0.2, "2026-06-06", "Cashflow"),
		fixtures.Dividend("d2", "RDDY", 100, 0.2, "2026-07-06", "Cashflow"),
	)
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-09-07", nil)
	months := BuildView(base, map[string]any{}).Cashflow.Months
	got := [][]any{}
	for _, m := range months {
		got = append(got, []any{m.Key, m.Count})
	}
	if !sameJSON(got, [][]any{{"2026-06", 1}, {"2026-07", 1}, {"2026-08", 0}, {"2026-09", 0}}) {
		t.Errorf("empty bars up to the current month: %s", jsonOf(got))
	}
	tiles := map[string]map[string]any{}
	for _, tile := range BuildView(base, map[string]any{}).Cashflow.Tiles {
		if _, ok := tile["perMonth"]; ok {
			tiles[tile["label"].(string)] = tile
		}
	}
	if tiles["2026 YTD"]["perMonth"] != 20.0 {
		t.Errorf("the monthly average counts paying months only: %v", tiles["2026 YTD"]["perMonth"])
	}
	months = BuildView(base, map[string]any{"to": "2026-08-15"}).Cashflow.Months
	keys := []string{}
	for _, m := range months {
		keys = append(keys, m.Key)
	}
	if !reflect.DeepEqual(keys, []string{"2026-06", "2026-07", "2026-08"}) {
		t.Errorf("a date filter ends the chart at its bound: %v", keys)
	}
	base25 := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2027-03-01", nil)
	months = BuildView(base25, map[string]any{"years": anyList("2026")}).Cashflow.Months
	if months[len(months)-1].Key != "2026-12" {
		t.Errorf("a year filter ends the chart at December: %q", months[len(months)-1].Key)
	}
}

func TestCashflowTilesRollOverWithTheCalendar(t *testing.T) {
	snapshot := snap(
		buy("b1", "RDDY", 100, 5, "2025-06-01", fixtures.WithAccount("Cashflow", "")),
		fixtures.Dividend("d1", "RDDY", 100, 0.2, "2025-07-06", "Cashflow"),
		fixtures.Dividend("d2", "RDDY", 100, 0.2, "2026-07-06", "Cashflow"),
	)
	tilesOn := func(today string) []map[string]any {
		return BuildView(BuildBase(snapshot, mkt(nil, nil), noJournal(), today, nil), map[string]any{}).Cashflow.Tiles
	}
	if got := tileLabels(tilesOn("2026-09-07")); !reflect.DeepEqual(got, []string{"2024", "2025", "2026 YTD", "All time", "Last 12 months", "Yield on cost"}) {
		t.Errorf("no margin account: Last 12 months stands in: %v", got)
	}
	if got := tileLabels(tilesOn("2027-01-01")); !reflect.DeepEqual(got, []string{"2025", "2026", "2027 YTD", "All time", "Last 12 months", "Yield on cost"}) {
		t.Errorf("labels: %v", got)
	}
	totals := map[string]float64{}
	for _, tile := range tilesOn("2027-01-01") {
		if total, ok := tile["total"]; ok {
			totals[tile["label"].(string)] = math.Round(total.(float64))
		}
	}
	if totals["2026"] != 20 || totals["2027 YTD"] != 0 || totals["All time"] != 40 {
		t.Errorf("totals: %v", totals)
	}
}

func TestWithoutAMarginAccountCashDayChangeAndLastTwelveMonthsStandIn(t *testing.T) {
	dividend := func(id, day string, per, cash float64) store.Activity {
		return row(store.Activity{ID: id, Category: "dividend", ActivityType: "Dividend", ActivitySubType: "dividend", RawType: "DIVIDEND", Quantity: 10, UnitPrice: per, NetCashAmount: cash, TransactionDate: day, Symbol: "AAA", Currency: "CAD", AccountType: "Trading"})
	}
	snapshot := &store.Snapshot{
		Activities: []store.Activity{
			buy("b1", "AAA", 10, 10, "2025-01-05", fixtures.WithAccount("Trading", "")),
			buy("b2", "BBB", 5, 20, "2025-01-06", fixtures.WithAccount("Kids", "acct-2"), fixtures.WithCurrency("USD")),
			dividend("d0", "2024-12-01", 1.0, 10),
			dividend("d1", "2025-06-01", 1.0, 10),
			dividend("d2", "2025-12-01", 1.5, 15),
		},
		Accounts: []store.Account{
			{ID: "acct-1", Nickname: "Trading", Currency: "CAD", NetLiquidationValue: ptr(1500.0), UnifiedAccountType: "SELF_DIRECTED_TFSA"},
			{ID: "acct-2", Nickname: "Kids", Currency: "CAD", NetLiquidationValue: ptr(500.0), UnifiedAccountType: "SELF_DIRECTED_RESP"},
		},
		Balances:   []store.Balance{{AccountID: "acct-1", SecurityID: "sec-c-cad", Quantity: ptr(300.0)}, {AccountID: "acct-2", SecurityID: "sec-c-usd", Quantity: ptr(10.0)}},
		Securities: []store.Security{{ID: "sec-c-cad", Symbol: "CAD", Currency: "CAD"}, {ID: "sec-c-usd", Symbol: "USD", Currency: "USD"}},
	}
	market := &store.MarketData{
		FX:     map[string]float64{"2026-02-01": 1.5},
		Quotes: map[string]store.Quote{"AAA": {Price: ptr(12.0), PriceChange: ptr(0.5), PercentChange: ptr(4.35)}, "BBB": {Price: ptr(30.0), PriceChange: ptr(-1.0), PercentChange: ptr(-3.2)}},
	}
	base := BuildBase(snapshot, market, noJournal(), "2026-02-01", nil)
	v := BuildView(base, nil)
	pf := v.Portfolio
	if pf.HasMargin {
		t.Error("hasMargin")
	}
	almost(t, pf.Cash, 300+10*1.5, "cash")
	almost(t, deref(pf.CashPct), 315.0/2000, "cash pct")
	almost(t, deref(pf.DayChange), 5-7.5, "day change")
	almost(t, deref(pf.DayChangePct), -2.5/(120+225+2.5), "day change pct")
	tiles := tileByLabel(v.Cashflow.Tiles)
	if _, ok := tiles["Margin used"]; ok {
		t.Error("Margin used tile present")
	}
	almost(t, tiles["Last 12 months"]["total"].(float64), 25, "the two payments since 2025-02-01")
	almost(t, tiles["Last 12 months"]["perMonth"].(float64), 12.5, "per month")
	if tiles["Last 12 months"]["count"] != 2 {
		t.Errorf("count: %v", tiles["Last 12 months"]["count"])
	}
	snapshot.Accounts[1].UnifiedAccountType = "SELF_DIRECTED_NON_REGISTERED_MARGIN"
	base = BuildBase(snapshot, market, noJournal(), "2026-02-01", nil)
	if !BuildView(base, nil).Portfolio.HasMargin {
		t.Error("a margin account in scope: hasMargin")
	}
	if BuildView(base, lists("account", "Trading")).Portfolio.HasMargin {
		t.Error("the TFSA alone: no margin")
	}
}

func TestMarginUsedTileAveragesInterestChargesOverChargedMonths(t *testing.T) {
	charge := func(id, day string, amount float64, ccy string) store.Activity {
		return bare(store.Activity{ID: id, ActivityType: "INTEREST_CHARGE", ActivitySubType: "MARGIN_INTEREST", RawType: "INTEREST_CHARGE", Category: "other", NetCashAmount: -amount, TransactionDate: day, Currency: ccy, AccountType: "Trading"})
	}
	snapshot := &store.Snapshot{
		Activities: []store.Activity{
			buy("b1", "AAA", 10, 10, "2026-01-05", fixtures.WithAccount("Trading", "")),
			charge("i1", "2026-07-01", 100, "CAD"),
			charge("i2", "2026-08-01", 20, "USD"),
			charge("i3", "2026-08-04", 10, "CAD"),
		},
		Accounts:   []store.Account{{ID: "acct-1", Nickname: "Trading", Currency: "CAD", NetLiquidationValue: ptr(1500.0), UnifiedAccountType: "SELF_DIRECTED_NON_REGISTERED_MARGIN"}},
		Balances:   []store.Balance{{AccountID: "acct-1", SecurityID: "sec-c-cad", Quantity: ptr(-300.0)}},
		Securities: []store.Security{{ID: "sec-c-cad", Symbol: "CAD", Currency: "CAD"}, {ID: "sec-c-usd", Symbol: "USD", Currency: "USD"}},
	}
	base := BuildBase(snapshot, mkt(map[string]float64{"2026-08-01": 1.5}, nil), noJournal(), "2026-09-06", nil)
	v := BuildView(base, nil)
	labels := tileLabels(v.Cashflow.Tiles)
	if got := labels[len(labels)-3:]; !reflect.DeepEqual(got, []string{"All time", "Margin used", "Yield on cost"}) {
		t.Errorf("labels: %v", got)
	}
	tile := v.Cashflow.Tiles[len(v.Cashflow.Tiles)-2]
	almost(t, tile["marginUsed"].(float64), v.Portfolio.MarginUsed, "margin used matches the portfolio")
	almost(t, tile["marginUsed"].(float64), 300.0, "margin used")
	if tile["interestMonths"] != 2 {
		t.Errorf("interest months: %v", tile["interestMonths"])
	}
	almost(t, tile["interestPerMonth"].(float64), (100+30+10)/2.0, "interest per month")
	v = BuildView(base, lists("account", "Cashflow"))
	labels = tileLabels(v.Cashflow.Tiles)
	if got := labels[len(labels)-2:]; !reflect.DeepEqual(got, []string{"Last 12 months", "Yield on cost"}) {
		t.Errorf("labels: %v", got)
	}
}

func TestYieldOnCostFromDeclaredRate(t *testing.T) {
	div := func(id, day string, qty, per float64) store.Activity {
		return row(store.Activity{ID: id, Category: "dividend", ActivityType: "Dividend", ActivitySubType: "dividend", RawType: "DIVIDEND", Quantity: qty, UnitPrice: per, NetCashAmount: qty * per, TransactionDate: day, Symbol: "RDDY", Currency: "CAD", AccountType: "Cashflow"})
	}
	snapshot := snap(
		buy("b1", "RDDY", 20000, 7.13, "2026-01-05", fixtures.WithAccount("Cashflow", "")),
		div("d1", "2026-07-06", 19000, 0.2),
		div("d2", "2026-08-06", 19000, 0.2),
		bare(store.Activity{ID: "int", Category: "interest", ActivityType: "Interest", RawType: "INTEREST", NetCashAmount: 4.5, TransactionDate: "2026-08-01", AccountType: "Cash"}),
		bare(store.Activity{ID: "wht", ActivityType: "WITHHOLDING_TAX", RawType: "WITHHOLDING_TAX", NetCashAmount: -40, TransactionDate: "2026-08-07", AccountType: "Cashflow"}),
	)
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-09-06", nil)
	if len(base.Cashflow) != 4 {
		t.Fatalf("cashflow: %s", jsonOf(base.Cashflow))
	}
	v := BuildView(base, nil)
	cf := v.Cashflow
	if cf.Count != 2 {
		t.Errorf("count: %d", cf.Count)
	}
	kinds := []string{}
	for _, r := range cf.Rows {
		kinds = append(kinds, r.Kind)
	}
	if !reflect.DeepEqual(kinds, []string{"Dividend", "Dividend"}) {
		t.Errorf("row kinds: %v", kinds)
	}
	other := map[string]bool{}
	for _, r := range cf.Other {
		other[r.Kind] = true
	}
	if !reflect.DeepEqual(sortedSet(other), []string{"Interest", "Withholding tax"}) {
		t.Errorf("other kinds: %v", other)
	}
	almost(t, cf.Total, 7600, "total")
	keys := []string{}
	for _, m := range cf.Months {
		keys = append(keys, m.Key)
	}
	if !reflect.DeepEqual(keys, []string{"2026-07", "2026-08", "2026-09"}) {
		t.Errorf("runs to the current month: %v", keys)
	}
	h := cf.Holdings[0]
	if h.Symbol != "RDDY" {
		t.Errorf("symbol: %q", h.Symbol)
	}
	if h.Freq == nil || *h.Freq != 12 {
		t.Errorf("freq: %v", h.Freq)
	}
	almost(t, deref(h.Yoc), 2.4/7.13, "yield on cost")
	almost(t, deref(h.Yob), 0.2*20000, "yield on book")
	almost(t, h.YTD, 7600, "ytd")
	tiles := tileByLabel(cf.Tiles)
	almost(t, tiles["2026 YTD"]["total"].(float64), 7600, "ytd total")
	almost(t, tiles["2026 YTD"]["perMonth"].(float64), 3800, "ytd per month")
	almost(t, deref(tiles["Yield on cost"]["yield"].(*float64)), (2.4*20000)/(20000*7.13), "yield")
	almost(t, tiles["Yield on cost"]["projected"].(float64), 2.4*20000/12, "projected")
	v = BuildView(base, lists("grade", "A"))
	if !hasFlag(v.Cashflow.SkippedFilters, "grade") {
		t.Errorf("skipped filters: %v", v.Cashflow.SkippedFilters)
	}
	if v.Cashflow.Count != 2 {
		t.Errorf("count: %d", v.Cashflow.Count)
	}
}

func TestFrequencyIsVerifiedFromDates(t *testing.T) {
	check := func(dates []string, want int) {
		t.Helper()
		got := PaymentsPerYear(dates)
		if got == nil || *got != want {
			t.Errorf("%v: got %v, want %d", dates, got, want)
		}
	}
	check([]string{"2026-07-06", "2026-08-06"}, 12)
	check([]string{"2026-08-06", "2026-07-06", "2026-06-05", "2026-05-06"}, 12)
	check([]string{"2025-01-07", "2026-01-07"}, 1)
	check([]string{"2025-03-20", "2025-06-20", "2025-09-22", "2025-12-19"}, 4)
	check([]string{"2026-01-02", "2026-01-09", "2026-01-16"}, 52)
	check([]string{"2026-01-06", "2026-02-06", "2026-03-06", "2026-04-06", "2026-05-06", "2026-05-13", "2026-05-20", "2026-05-27"}, 52)
	if got := PaymentsPerYear([]string{"2026-08-06"}); got != nil {
		t.Errorf("one payment: %d", *got)
	}
	if got := PaymentsPerYear([]string{"2026-08-06", "2026-08-06"}); got != nil {
		t.Errorf("one day twice: %d", *got)
	}
}

func TestFrequencyUsesPaymentRowsWithoutPerUnitValues(t *testing.T) {
	div := func(id, day string, qty, per, amount float64) store.Activity {
		return row(store.Activity{ID: id, Category: "dividend", ActivityType: "Dividend", ActivitySubType: "dividend", RawType: "DIVIDEND", Quantity: qty, UnitPrice: per, NetCashAmount: amount, TransactionDate: day, Symbol: "VEQT", Currency: "CAD", AccountType: "Kids"})
	}
	snapshot := snap(buy("b1", "VEQT", 300, 49.76, "2024-06-01", fixtures.WithAccount("Kids", "")), div("v1", "2025-01-07", 0, 0, 91.56), div("v2", "2026-01-07", 300, 0.76, 228))
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-09-06", nil)
	h := BuildView(base, nil).Cashflow.Holdings[0]
	if h.Freq == nil || *h.Freq != 1 {
		t.Errorf("freq: %v", h.Freq)
	}
	if !h.FreqVerified {
		t.Error("freq not verified")
	}
}

func TestSinglePaymentShowsNoYieldAndAnnualPayerIsNotX12(t *testing.T) {
	div := func(id, sym, day string, qty, per float64) store.Activity {
		return row(store.Activity{ID: id, Category: "dividend", ActivityType: "Dividend", ActivitySubType: "dividend", RawType: "DIVIDEND", Quantity: qty, UnitPrice: per, NetCashAmount: qty * per, TransactionDate: day, Symbol: sym, Currency: "CAD", AccountType: "Kids"})
	}
	kids := fixtures.WithAccount("Kids", "")
	snapshot := snap(
		buy("b1", "VEQT", 300, 49.76, "2024-06-01", kids),
		div("dVEQT1", "VEQT", "2025-01-07", 300, 0.76),
		div("dVEQT2", "VEQT", "2026-01-07", 300, 0.76),
		buy("b2", "NEWM", 1000, 10.0, "2026-07-01", kids),
		div("dNEWM1", "NEWM", "2026-08-06", 1000, 0.1),
	)
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-09-06", nil)
	h := map[string]CashHolding{}
	for _, x := range BuildView(base, nil).Cashflow.Holdings {
		h[x.Symbol] = x
	}
	if h["VEQT"].Freq == nil || *h["VEQT"].Freq != 1 {
		t.Errorf("VEQT freq: %v", h["VEQT"].Freq)
	}
	if !h["VEQT"].FreqVerified {
		t.Error("VEQT freq not verified")
	}
	almost(t, deref(h["VEQT"].Yoc), 0.76/49.76, "VEQT yield on cost")
	if h["NEWM"].Freq == nil || *h["NEWM"].Freq != 12 {
		t.Errorf("NEWM freq: %v", h["NEWM"].Freq)
	}
	if h["NEWM"].FreqVerified {
		t.Error("NEWM freq verified")
	}
	almost(t, deref(h["NEWM"].Yoc), 0.1*12/10.0, "NEWM yield on cost")
	if deref(h["NEWM"].Per) != 0.1 {
		t.Errorf("NEWM per: %v", h["NEWM"].Per)
	}
}

func TestDeclaredRecordBeatsOwnHistoryAndTracksScheduleChange(t *testing.T) {
	div := func(id, day string, qty, per float64) store.Activity {
		return row(store.Activity{ID: id, Category: "dividend", ActivityType: "Dividend", ActivitySubType: "dividend", RawType: "DIVIDEND", Quantity: qty, UnitPrice: per, NetCashAmount: qty * per, TransactionDate: day, Symbol: "CCHI", Currency: "CAD", AccountType: "Cashflow"})
	}
	snapshot := snap(buy("b1", "CCHI", 4000, 11.64, "2026-08-25", fixtures.WithAccount("Cashflow", "")), div("c1", "2026-09-04", 4000, 0.135))
	public := map[string][]store.Distribution{"CCHI": {
		{ExDate: "2026-09-15", PayDate: "2026-09-21", Amount: 0.135, Currency: "CAD"},
		{ExDate: "2026-08-31", PayDate: "2026-09-04", Amount: 0.135, Currency: "CAD"},
		{ExDate: "2026-08-14", PayDate: "2026-08-20", Amount: 0.135, Currency: "CAD"},
		{ExDate: "2026-07-31", PayDate: "2026-08-10", Amount: 0.27, Currency: "CAD"},
		{ExDate: "2026-06-30", PayDate: "2026-07-08", Amount: 0.27, Currency: "CAD"},
		{ExDate: "2026-05-29", PayDate: "2026-06-05", Amount: 0.27, Currency: "CAD"},
	}}
	quotes := map[string]store.Quote{"CCHI": {Price: ptr(10.95), DividendAmount: ptr(0.135), DividendFrequency: "", ExDividendDate: "2026-09-15", FetchedAt: "2026-09-06T00:00:00Z"}}
	base := BuildBase(snapshot, &store.MarketData{Distributions: public, Quotes: quotes}, noJournal(), "2026-09-06", nil)
	h := BuildView(base, nil).Cashflow.Holdings[0]
	if deref(h.Per) != 0.135 {
		t.Errorf("per: %v", h.Per)
	}
	if h.Freq == nil || *h.Freq != 24 {
		t.Errorf("freq: %v", h.Freq)
	}
	if !h.FreqVerified {
		t.Error("freq not verified")
	}
	if h.RateSource != "declared" {
		t.Errorf("rate source: %q", h.RateSource)
	}
	almost(t, deref(h.Yoc), 0.135*24/11.64, "yield on cost")
	if h.Last != 10.95 {
		t.Errorf("last: %v", h.Last)
	}
	if h.PriceSource != "close" {
		t.Errorf("price source: %q", h.PriceSource)
	}
	almost(t, deref(h.CurrentYield), 0.135*24/10.95, "current yield")
	base = BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-09-06", nil)
	h = BuildView(base, nil).Cashflow.Holdings[0]
	if h.RateSource != "payments" {
		t.Errorf("rate source: %q", h.RateSource)
	}
	if h.FreqVerified {
		t.Error("freq verified from a single payment")
	}
	if h.PriceSource != "fill" {
		t.Errorf("price source: %q", h.PriceSource)
	}
}

func TestPayerSymbolsAreHeldDividendPayers(t *testing.T) {
	cashflow := fixtures.WithAccount("Cashflow", "")
	snapshot := snap(
		buy("b1", "RDDY", 100, 7, "2026-01-05", cashflow),
		fixtures.Dividend("d1", "RDDY", 100, 0.2, "2026-02-06", "Cashflow"),
		buy("b2", "TD", 10, 80, "2025-01-05", cashflow),
		fixtures.Dividend("d2", "TD", 10, 1, "2025-02-06", "Cashflow"),
		sell("s2", "TD", 10, 90, "2025-03-01", cashflow),
		buy("b3", "AAA", 10, 5, "2026-01-05", cashflow),
	)
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-09-06", nil)
	got := PayerSymbols(base)
	if !reflect.DeepEqual(got, []PayerSymbol{{"RDDY", "", "CAD"}}) {
		t.Errorf("payers: %s", jsonOf(got))
	}
}
