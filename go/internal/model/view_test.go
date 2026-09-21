package model

import (
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func viewFixture() (*store.Snapshot, *store.MarketData, map[string]store.JournalEntry) {
	trading := fixtures.WithAccount("Trading", "")
	retirement := fixtures.WithAccount("Retirement", "")
	snapshot := &store.Snapshot{
		Activities: []store.Activity{
			buy("b1", "AAA", 100, 10, "2025-03-01", trading),
			sell("s1", "AAA", 100, 12, "2025-03-10", trading),
			buy("b2", "BBB", 10, 100, "2026-01-05", trading),
			sell("s2", "BBB", 10, 90, "2026-01-20", trading),
			buy("b3", "CCC", 10, 5, "2026-02-01", retirement),
			sell("s3", "CCC", 10, 6, "2026-02-15", retirement),
			buy("b4", "DDD", 10, 5, "2026-03-01", trading),
			buy("b5", "LUNR", 10, 10, "2026-03-01", trading, fixtures.WithCurrency("USD")),
			sell("s5", "LUNR", 10, 11, "2026-03-05", trading, fixtures.WithCurrency("USD")),
		},
		Accounts: []store.Account{
			{ID: "acct-1", Nickname: "Trading", UnifiedAccountType: "TFSA", Currency: "CAD"},
			{ID: "acct-2", Nickname: "Retirement", UnifiedAccountType: "RRSP", Currency: "CAD"},
		},
		NavHistory: []store.NavPoint{
			{Date: "2024-12-31", Equity: 1000, NetDeposits: ptr(1000)},
			{Date: "2025-06-30", Equity: 1500, NetDeposits: ptr(1200)},
			{Date: "2025-12-31", Equity: 1600, NetDeposits: ptr(1200)},
			{Date: "2026-03-31", Equity: 1400, NetDeposits: ptr(1200)},
		},
		NavByAccount: map[string][]store.NavPoint{"Trading": {{Date: "2025-12-31", Equity: 800, NetDeposits: ptr(500)}, {Date: "2026-03-31", Equity: 700, NetDeposits: ptr(500)}}},
		SyncedAt:     "2026-04-01T00:00:00Z",
	}
	market := mkt(map[string]float64{"2026-03-01": 1.4, "2026-03-05": 1.3}, map[string]float64{"2024-12-31": 100, "2025-12-31": 120, "2026-03-31": 126})
	journal := map[string]store.JournalEntry{
		"rt:b1": {Thesis: "yes", Tags: []string{"x"}, Grade: "A"},
		"rt:b2": {Thesis: "", Tags: []string{}, Grade: "F"},
	}
	return snapshot, market, journal
}

func viewBase() *Base {
	snapshot, market, journal := viewFixture()
	return BuildBase(snapshot, market, journal, "2026-04-01", nil)
}

func TestOneListFeedsEveryTile(t *testing.T) {
	v := BuildView(viewBase(), nil)
	k := v.KPI
	if k.Count != 4 {
		t.Errorf("count: %d", k.Count)
	}
	if len(v.Trades) != 4 {
		t.Errorf("trades: %d", len(v.Trades))
	}
	realized := 0.0
	for _, tr := range v.Trades {
		realized += tr.PnlCad
	}
	almost(t, k.Realized, realized, "realized")
	monthly := 0.0
	for _, m := range v.Monthly {
		monthly += m.Value
	}
	almost(t, monthly, k.Realized, "monthly sum")
	bySym, bySymN := 0.0, 0
	for _, r := range v.BySymbol {
		bySym += r.Pnl
		bySymN += r.N
	}
	almost(t, bySym, k.Realized, "by-symbol sum")
	g := v.Grades
	graded := 0
	for _, b := range g.Buckets {
		graded += b.N
	}
	if graded+g.Ungraded != k.Count {
		t.Errorf("grades: %d + %d != %d", graded, g.Ungraded, k.Count)
	}
	if bySymN != k.Count {
		t.Errorf("by-symbol count: %d", bySymN)
	}
	if k.Wins+k.Losses+k.Breakeven != k.Count {
		t.Errorf("wins %d losses %d breakeven %d", k.Wins, k.Losses, k.Breakeven)
	}
	if len(v.Queue) != 3 {
		t.Errorf("queue: %s", jsonOf(v.Queue))
	}
	var usd *Trade
	for _, tr := range v.Trades {
		if tr.Symbol == "LUNR" {
			usd = tr
		}
	}
	if usd == nil {
		t.Fatal("no LUNR trade")
	}
	almost(t, usd.Pnl, 10, "usd pnl")
	almost(t, usd.PnlCad, 110*1.3-100*1.4, "usd pnl in CAD")
}

func TestPositionsAndOptions(t *testing.T) {
	v := BuildView(viewBase(), nil)
	if len(v.Positions) != 1 {
		t.Fatalf("positions: %s", jsonOf(v.Positions))
	}
	p := v.Positions[0]
	if p.Symbol != "DDD" {
		t.Errorf("symbol: %q", p.Symbol)
	}
	if p.PriceSource != "fill" {
		t.Errorf("priceSource: %q", p.PriceSource)
	}
	if p.Alloc != 1.0 {
		t.Errorf("alloc: %v", p.Alloc)
	}
	if p.Held != 31 {
		t.Errorf("held: %d", p.Held)
	}
	if !reflect.DeepEqual(v.Options.Accounts, []string{"Retirement", "Trading"}) {
		t.Errorf("accounts: %v", v.Options.Accounts)
	}
	if !reflect.DeepEqual(v.Options.Kinds, []string{"Shares"}) {
		t.Errorf("kinds: %v", v.Options.Kinds)
	}
	if !reflect.DeepEqual(v.Options.Tags, []string{"x"}) {
		t.Errorf("tags: %v", v.Options.Tags)
	}
	listings := v.Options.Listings
	keys := []string{}
	for k := range listings {
		keys = append(keys, k)
	}
	if !reflect.DeepEqual(sortedStrings(keys), v.Options.Symbols) {
		t.Errorf("one listing per symbol the ⌘K list can show: %v vs %v", sortedStrings(keys), v.Options.Symbols)
	}
	if listings["DDD"].Exchange != p.Exchange {
		t.Errorf("exchange: %q vs %q", listings["DDD"].Exchange, p.Exchange)
	}
	if listings["DDD"].Kind != "Shares" {
		t.Errorf("kind: %q", listings["DDD"].Kind)
	}
	if listings["DDD"] == nil {
		t.Error("the DDD listing carries a name")
	}
}

func TestExDivAndPayDayNextDeclaredElseLastKnown(t *testing.T) {
	cashflow := fixtures.WithAccount("Cashflow", "")
	acts := []store.Activity{
		buy("b1", "RDDY", 100, 5, "2026-05-01", cashflow),
		fixtures.Dividend("d1", "RDDY", 100, 0.2, "2026-08-06", "Cashflow"),
		buy("b2", "HHIS", 100, 5, "2026-05-01", cashflow),
		fixtures.Dividend("d2", "HHIS", 100, 0.2, "2026-08-06", "Cashflow"),
		buy("b3", "HBIX", 100, 5, "2026-05-01", cashflow),
		fixtures.Dividend("d3", "HBIX", 100, 0.2, "2026-08-06", "Cashflow"),
		buy("b4", "EASY", 100, 20, "2026-05-01", cashflow),
		fixtures.Dividend("d4", "EASY", 100, 0.31, "2026-08-21", "Cashflow"),
	}
	snapshot := snap(acts...)
	market := &store.MarketData{
		Distributions: map[string][]store.Distribution{
			"RDDY": {{ExDate: "2026-09-30", PayDate: "2026-10-06", Amount: 0.15, Currency: "CAD"}, {ExDate: "2026-08-31", PayDate: "2026-09-04", Amount: 0.15, Currency: "CAD"}},
			"HHIS": {{ExDate: "2026-08-31", PayDate: "2026-09-04", Amount: 0.27, Currency: "CAD"}},
			"EASY": {{ExDate: "2026-08-31", PayDate: "2026-09-08", Amount: 0.255, Currency: "CAD"}, {ExDate: "2026-09-15", PayDate: "2026-09-22", Amount: 0.255, Currency: "CAD"}},
		},
		Quotes: map[string]store.Quote{
			"HHIS": {Price: ptr(11.0), ExDividendDate: "2026-09-29"},
			"HBIX": {Price: ptr(6.7), ExDividendDate: "2026-08-29"},
		},
	}
	type dates struct {
		ex, pay         string
		exPast, payPast bool
	}
	datesOn := func(today string) map[string]dates {
		base := BuildBase(snapshot, market, noJournal(), today, nil)
		out := map[string]dates{}
		for _, h := range BuildView(base, map[string]any{}).Cashflow.Holdings {
			out[h.Symbol] = dates{h.NextExDate, h.NextPayDate, h.ExPast, h.PayPast}
		}
		return out
	}
	by := datesOn("2026-09-07")
	if by["RDDY"] != (dates{"2026-09-30", "2026-10-06", false, false}) {
		t.Errorf("the declared record's next distribution, with its pay date: %+v", by["RDDY"])
	}
	if by["EASY"] != (dates{"2026-08-31", "2026-09-08", true, false}) {
		t.Errorf("gone ex but not yet paid: that distribution, not the one after it: %+v", by["EASY"])
	}
	if by["HHIS"] != (dates{"2026-08-31", "2026-09-04", true, true}) {
		t.Errorf("nothing left to pay: the last known one, both dates passed: %+v", by["HHIS"])
	}
	if by["HBIX"] != (dates{"2026-08-29", "2026-08-06", true, true}) {
		t.Errorf("no record: the quote's last ex-date and the last payment received: %+v", by["HBIX"])
	}
	by = datesOn("2026-09-08")
	if by["EASY"] != (dates{"2026-08-31", "2026-09-08", true, false}) {
		t.Errorf("pay day itself still counts as ahead: %+v", by["EASY"])
	}
	by = datesOn("2026-09-09")
	if by["EASY"] != (dates{"2026-09-15", "2026-09-22", false, false}) {
		t.Errorf("once paid, the next one: %+v", by["EASY"])
	}
}
