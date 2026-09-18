package model

import (
	"sort"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func TestMultilegZeroQtyClosesShort(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -16, UnitPrice: 6.2225, NetCashAmount: 9956, TransactionDate: "2026-01-10"}),
		row(store.Activity{ID: "ml1", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: -128, TransactionDate: "2026-03-01"}),
		row(store.Activity{ID: "ml2", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: -2025, TransactionDate: "2026-03-01"}),
	}))
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	var real []*Slice
	for _, s := range r.Closed {
		if !hasFlag(s.Flags, "rolled-out") {
			real = append(real, s)
		}
	}
	if len(real) != 2 {
		t.Fatalf("real closed: %s", jsonOf(real))
	}
	sort.SliceStable(real, func(i, j int) bool { return real[i].Quantity < real[j].Quantity })
	if real[0].Quantity != 1 {
		t.Errorf("quantity: got %v, want 1", real[0].Quantity)
	}
	almost(t, real[0].ExitPrice, 1.28, "exit price of the single contract")
	if real[1].Quantity != 15 {
		t.Errorf("quantity: got %v, want 15", real[1].Quantity)
	}
	almost(t, real[1].ExitPrice, 1.35, "exit price of the fifteen")
	for _, s := range r.Closed {
		if s.OpenDirection != "SHORT" {
			t.Errorf("open direction: %s", jsonOf(s))
		}
	}
	want := (6.2225-1.28)*1*100 + (6.2225-1.35)*15*100
	almost(t, sumPnl(r.Closed), want, "pnl")
	for _, s := range real {
		if s.RT != "rt:sto" {
			t.Errorf("rt: got %q, want rt:sto", s.RT)
		}
	}
}

func TestRollCarriesTheUnpostedLegToTheNextBuyBack(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -16, UnitPrice: 6.2225, NetCashAmount: 9956, TransactionDate: "2025-10-01", Symbol: "LUNR 15JAN27 12.00 CALL"}),
		row(store.Activity{ID: "ml", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: -2160, TransactionDate: "2025-11-14", Symbol: "LUNR 15JAN27 12.00 CALL"}),
		row(store.Activity{ID: "sto2", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -6, UnitPrice: 6.75, NetCashAmount: 4050, TransactionDate: "2025-12-10", Symbol: "LUNR 21JAN28 12.00 CALL"}),
		row(store.Activity{ID: "btc", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 22, UnitPrice: 13.3, NetCashAmount: -29260, TransactionDate: "2026-06-26", Symbol: "LUNR 21JAN28 12.00 CALL"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	almost(t, sumPnl(r.Closed), 9956-2160+4050-29260, "total pnl")
	rolledQty := 0.0
	for _, s := range r.Closed {
		if hasFlag(s.Flags, "rolled-in") {
			rolledQty += s.Quantity
			if s.Symbol != "LUNR 21JAN28 12.00 CALL" {
				t.Errorf("rolled-in symbol: %q", s.Symbol)
			}
		}
	}
	almost(t, rolledQty, 16, "rolled-in quantity")
	jan28 := map[string]bool{}
	for _, s := range r.Closed {
		if s.Symbol == "LUNR 21JAN28 12.00 CALL" {
			jan28[s.RT] = true
		}
	}
	if len(jan28) != 1 {
		t.Errorf("jan28 round trips: %v", jan28)
	}
}

func TestCreditRollUpMovesShortsToTheNewStrike(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -5, UnitPrice: 3.0, NetCashAmount: 1500, TransactionDate: "2025-11-12", Symbol: "BBAI 21JAN28 10.00 CALL"}),
		row(store.Activity{ID: "cr1", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: 14, TransactionDate: "2026-06-09", Symbol: "BBAI 21JAN28 10.00 CALL"}),
		row(store.Activity{ID: "cr2", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: 56, TransactionDate: "2026-06-17", Symbol: "BBAI 21JAN28 10.00 CALL"}),
		row(store.Activity{ID: "btc", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 5, UnitPrice: 0.85, NetCashAmount: -425, TransactionDate: "2026-06-26", Symbol: "BBAI 21JAN28 12.00 CALL"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	almost(t, sumPnl(r.Closed), 1500+14+56-425, "total pnl")
}

func TestBuyBackClosesOlderContractsOfARolledChain(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "s1", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -3, UnitPrice: 0.12, NetCashAmount: 36, TransactionDate: "2025-12-05", Symbol: "BBAI 26DEC25 5.50 PUT"}),
		row(store.Activity{ID: "s2", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -5, UnitPrice: 0.2, NetCashAmount: 100, TransactionDate: "2025-12-11", Symbol: "BBAI 02JAN26 5.50 PUT"}),
		row(store.Activity{ID: "s3", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -1, UnitPrice: 0.4, NetCashAmount: 40, TransactionDate: "2025-12-15", Symbol: "BBAI 26DEC25 6.00 PUT"}),
		row(store.Activity{ID: "s4", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -6, UnitPrice: 0.2, NetCashAmount: 120, TransactionDate: "2025-12-12", Symbol: "BBAI 19DEC25 6.00 PUT"}),
		row(store.Activity{ID: "ml1", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: -18, TransactionDate: "2025-12-15", Symbol: "BBAI 19DEC25 6.00 PUT"}),
		row(store.Activity{ID: "ml2", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: -1830, TransactionDate: "2025-12-18", Symbol: "BBAI 18JUN26 5.00 PUT"}),
		row(store.Activity{ID: "s5", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -11, UnitPrice: 2.4, NetCashAmount: 2640, TransactionDate: "2026-02-27", Symbol: "BBAI 21JAN28 5.00 PUT"}),
		row(store.Activity{ID: "btc", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 26, UnitPrice: 2.74, NetCashAmount: -7124, TransactionDate: "2026-06-29", Symbol: "BBAI 21JAN28 5.00 PUT"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	almost(t, sumPnl(r.Closed), 36+100+40+120-18-1830+2640-7124, "total pnl")
	for _, s := range r.Closed {
		if s.ExitDate == "2026-06-29" && s.Symbol != "BBAI 21JAN28 5.00 PUT" {
			t.Errorf("closed on the buy-back day under %q", s.Symbol)
		}
	}
}

func TestPlainOptionBuysWithoutARollStayLong(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "bto", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 10, UnitPrice: 1.27, NetCashAmount: -1270, TransactionDate: "2026-06-15", Symbol: "QNC 20NOV26 3.00 CALL"}),
	}))
	if len(r.Open) != 1 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if r.Open[0].Direction != "LONG" {
		t.Errorf("direction: %q", r.Open[0].Direction)
	}
}

func TestShortExpiryClosesShort(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "sto2", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -5, UnitPrice: 2, NetCashAmount: 1000, TransactionDate: "2026-01-10", Symbol: "ABC 15JAN27 10.00 CALL"}),
		row(store.Activity{ID: "exp", ActivityType: "OPTIONS_SHORT_EXPIRY", ActivitySubType: "EXPIRED", RawType: "OPTIONS_SHORT_EXPIRY", Quantity: 5, TransactionDate: "2027-01-15", Symbol: "ABC 15JAN27 10.00 CALL"}),
	}))
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
	if r.Closed[0].ExitPrice != 0 {
		t.Errorf("exit price: %v", r.Closed[0].ExitPrice)
	}
	if r.Closed[0].Quantity != 5 {
		t.Errorf("quantity: %v", r.Closed[0].Quantity)
	}
	almost(t, r.Closed[0].Pnl, 1000, "pnl")
}

func TestSharesRoundTrip(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{buy("b", "AAA", 10, 12, "2026-01-10"), sell("s", "AAA", 10, 15, "2026-02-10")}))
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
	if r.Closed[0].Quantity != 10 {
		t.Errorf("quantity: %v", r.Closed[0].Quantity)
	}
	almost(t, r.Closed[0].Pnl, 30, "pnl")
	if r.Closed[0].HoldDays != 31 {
		t.Errorf("hold days: %d", r.Closed[0].HoldDays)
	}
	if isOption("AAA") {
		t.Error("AAA is not an option")
	}
	if !isOption("LUNR 15JAN27 12.00 CALL") {
		t.Error("LUNR 15JAN27 12.00 CALL is an option")
	}
}

func TestCreditMultilegsOnAShortAreARoll(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "bbai-sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -3, UnitPrice: 1.2, NetCashAmount: 360, TransactionDate: "2026-01-05", Symbol: "BBAI 21JAN28 10.00 CALL"}),
		row(store.Activity{ID: "bbai-cr1", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOCLOSE", RawType: "OPTIONS_MULTILEG", NetCashAmount: 14, TransactionDate: "2026-02-01", Symbol: "BBAI 21JAN28 10.00 CALL"}),
		row(store.Activity{ID: "bbai-cr2", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: 56, TransactionDate: "2026-02-01", Symbol: "BBAI 21JAN28 10.00 CALL"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	almost(t, sumPnl(r.Closed), 360+14+56, "total pnl")
}

func TestLongExpiryAndSameDayExpiry(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "lunr-bto", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 2, UnitPrice: 0.4, NetCashAmount: -80, TransactionDate: "2025-07-01", Symbol: "LUNR 22AUG25 8.00 CALL"}),
		row(store.Activity{ID: "lunr-exp", Category: "option_event", ActivityType: "EXPIR", ActivitySubType: "BUY", RawType: "OPTIONS_EXPIRY", Quantity: 2, TransactionDate: "2025-08-22", Symbol: "LUNR 22AUG25 8.00 CALL"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
	if r.Closed[0].OpenDirection != "LONG" {
		t.Errorf("open direction: %q", r.Closed[0].OpenDirection)
	}
	almost(t, r.Closed[0].Pnl, -80, "pnl")
	r = MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "spy-bto", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 1, UnitPrice: 1.1, NetCashAmount: -110, TransactionDate: "2025-07-17", Symbol: "SPY 17JUL25 624.00 PUT"}),
		row(store.Activity{ID: "spy-exp", ActivityType: "OPTIONS_EXPIRY", ActivitySubType: "EXPIRED", RawType: "OPTIONS_EXPIRY", Quantity: 1, TransactionDate: "2025-07-17", Symbol: "SPY 17JUL25 624.00 PUT"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
}

func TestDebitMultilegOpensLongAndStoOpensShort(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "put-ml", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", RawType: "OPTIONS_MULTILEG", NetCashAmount: -90, TransactionDate: "2026-01-30", Symbol: "BBAI 30JAN26 6.00 PUT"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 1 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if r.Open[0].Direction != "LONG" {
		t.Errorf("direction: %q", r.Open[0].Direction)
	}
	r = MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "sto-only", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -4, UnitPrice: 2, NetCashAmount: 800, TransactionDate: "2026-01-01", Symbol: "XYZ 15JAN27 5.00 CALL"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 1 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if r.Open[0].Direction != "SHORT" {
		t.Errorf("direction: %q", r.Open[0].Direction)
	}
	if r.Open[0].Qty != 4 {
		t.Errorf("qty: %v", r.Open[0].Qty)
	}
}

func TestAssignmentKeepsPremium(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "asts-sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -1, UnitPrice: 4.7475, NetCashAmount: 474.75, TransactionDate: "2025-01-15", Symbol: "ASTS 07MAR25 31.00 CALL"}),
		row(store.Activity{ID: "asts-asg", Category: "option_event", ActivityType: "ASSIGN", ActivitySubType: "BUYTOCLOSE", RawType: "OPTIONS_ASSIGN", Quantity: 1, UnitPrice: 31, NetCashAmount: -3100, TransactionDate: "2025-03-07", Symbol: "ASTS 07MAR25 31.00 CALL"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
	if r.Closed[0].ExitPrice != 0 {
		t.Errorf("exit price: %v", r.Closed[0].ExitPrice)
	}
	almost(t, r.Closed[0].Pnl, 474.75, "pnl")
}

func TestSameDayRollFoldsIntoFarContract(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		row(store.Activity{ID: "aug-sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -1, UnitPrice: 3, NetCashAmount: 300, TransactionDate: "2026-01-01", Symbol: "ZZZ 21AUG26 10.00 CALL"}),
		row(store.Activity{ID: "aug-cover", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOCLOSE", RawType: "OPTIONS_BUY", Quantity: 1, UnitPrice: 1, NetCashAmount: -100, TransactionDate: "2026-08-15", Symbol: "ZZZ 21AUG26 10.00 CALL"}),
		row(store.Activity{ID: "jan-sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -1, UnitPrice: 2, NetCashAmount: 200, TransactionDate: "2026-08-15", Symbol: "ZZZ 15JAN27 12.00 CALL"}),
		row(store.Activity{ID: "jan-cover", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOCLOSE", RawType: "OPTIONS_BUY", Quantity: 1, UnitPrice: 0.5, NetCashAmount: -50, TransactionDate: "2026-12-01", Symbol: "ZZZ 15JAN27 12.00 CALL"}),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
	if r.Closed[0].Symbol != "ZZZ 15JAN27 12.00 CALL" {
		t.Errorf("symbol: %q", r.Closed[0].Symbol)
	}
	almost(t, r.Closed[0].EntryPrice, 4, "entry price")
	almost(t, r.Closed[0].Pnl, 350, "pnl")
	if !hasFlag(r.Closed[0].Flags, "rolled") {
		t.Errorf("flags: %v", r.Closed[0].Flags)
	}
}

func TestStkdisNameChangeNetsToZero(t *testing.T) {
	r := MatchFIFO(ptrs([]store.Activity{
		buy("b", "OLD", 100, 2, "2026-01-01"),
		row(store.Activity{ID: "out", Category: "trade", ActivityType: "STKDIS", ActivitySubType: "SELL", RawType: "CORPORATE_ACTION", Quantity: -100, TransactionDate: "2026-02-01", Symbol: "OLD", Currency: "CAD"}),
		row(store.Activity{ID: "in", Category: "trade", ActivityType: "STKDIS", ActivitySubType: "BUY", RawType: "CORPORATE_ACTION", Quantity: 100, TransactionDate: "2026-02-01", Symbol: "NEW", Currency: "CAD"}),
		sell("s", "NEW", 100, 3, "2026-03-01"),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
	if r.Closed[0].Symbol != "NEW" {
		t.Errorf("symbol: %q", r.Closed[0].Symbol)
	}
	almost(t, r.Closed[0].Pnl, 300, "pnl")
	openSymbols := []string{}
	for _, l := range r.Open {
		openSymbols = append(openSymbols, l.Symbol)
	}
	if !sameJSON(openSymbols, []string{"OLD"}) {
		t.Errorf("open symbols: %v", openSymbols)
	}
	r = MatchFIFO(ptrs([]store.Activity{
		buy("b", "OLD", 100, 2, "2026-01-01"),
		row(store.Activity{ID: "out", Category: "trade", ActivityType: "STKDIS", ActivitySubType: "SELL", RawType: "CODE_CHANGE", Quantity: -100, TransactionDate: "2026-02-01", Symbol: "OLD", Currency: "CAD"}),
		sell("s", "NEW", 100, 3, "2026-03-01"),
	}))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(r.Closed))
	}
	if r.Closed[0].Symbol != "NEW" {
		t.Errorf("symbol: %q", r.Closed[0].Symbol)
	}
	almost(t, r.Closed[0].Pnl, 100, "pnl")
	if len(r.Open) != 0 {
		t.Errorf("open: %s", jsonOf(r.Open))
	}
}

func TestReverseSplitMarkerRescalesOpenLots(t *testing.T) {
	acts := []store.Activity{
		buy("b1", "MSTY", 100, 7.0, "2025-12-01"),
		buy("b2", "MSTY", 75, 6.9, "2025-12-05"),
		row(store.Activity{ID: "ca", Category: "trade", ActivityType: "STKDIS", ActivitySubType: "BUY", RawType: "CORPORATE_ACTION", TransactionDate: "2025-12-08", Symbol: "MSTY", Currency: "CAD"}),
		buy("b3", "MSTY", 4, 34.0, "2025-12-11"),
		sell("s1", "MSTY", 39, 31.0, "2026-01-16"),
	}
	r := MatchFIFO(NormalizeActivities(acts))
	if len(r.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(r.Unmatched))
	}
	if len(r.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(r.Open))
	}
	qty := 0.0
	for _, s := range r.Closed {
		qty += s.Quantity
	}
	almost(t, qty, 39, "closed quantity")
	first := r.Closed[0]
	for _, s := range r.Closed {
		if s.EntryDate < first.EntryDate {
			first = s
		}
	}
	almost(t, first.EntryPrice, 35.0, "first entry price")
	almost(t, sumPnl(r.Closed), 39*31-(100*7+75*6.9+4*34), "pnl")
}

func TestForwardSplitAndNoMarkerWithoutPrices(t *testing.T) {
	acts := []store.Activity{
		buy("b1", "NVDA", 10, 1000.0, "2024-05-01"),
		row(store.Activity{ID: "ca", Category: "trade", ActivityType: "STKDIS", ActivitySubType: "BUY", RawType: "CORPORATE_ACTION", TransactionDate: "2024-06-10", Symbol: "NVDA", Currency: "CAD"}),
		buy("b2", "NVDA", 5, 98.0, "2024-06-12", fixtures.WithCurrency("USD")),
	}
	acts[0].Currency = "USD"
	r := MatchFIFO(NormalizeActivities(acts))
	qty := 0.0
	for _, l := range r.Open {
		qty += l.Qty
	}
	almost(t, qty, 105, "open quantity")
	big := r.Open[0]
	for _, l := range r.Open {
		if l.Qty > big.Qty {
			big = l
		}
	}
	almost(t, big.Price, 100.0, "split price")
	if !hasFlag(big.Flags, "split 10:1") {
		t.Errorf("flags: %v", big.Flags)
	}
	r = MatchFIFO(NormalizeActivities([]store.Activity{
		buy("b1", "AAA", 10, 10.0, "2024-05-01"),
		row(store.Activity{ID: "ca", Category: "trade", ActivityType: "STKDIS", ActivitySubType: "BUY", RawType: "CORPORATE_ACTION", TransactionDate: "2024-06-10", Symbol: "AAA", Currency: "CAD"}),
	}))
	if r.Open[0].Qty != 10 {
		t.Errorf("qty: %v", r.Open[0].Qty)
	}
}

func TestATransferOutLeavesAtCostWithNoPnl(t *testing.T) {
	acts := []store.Activity{
		fixtures.Crypto("cb", "buy", "ETH", 2, 100, "2026-01-01", "Ponzi"),
		fixtures.CryptoTransfer("ti", "ETH", 1, 120, "2026-01-05", false, "Ponzi"),
		fixtures.CryptoTransfer("to", "ETH", 1, 200, "2026-01-10", true, "Ponzi"),
		fixtures.Crypto("cs", "sell", "ETH", 2, 150, "2026-02-01", "Ponzi"),
	}
	fifo := MatchFIFO(ptrs(acts))
	if len(fifo.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(fifo.Unmatched))
	}
	if len(fifo.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(fifo.Open))
	}
	got := [][3]float64{}
	for _, s := range fifo.Closed {
		got = append(got, [3]float64{py.Round(s.Pnl, 6), s.Quantity, s.EntryPrice})
	}
	if !sameJSON(got, [][3]float64{{50, 1, 100}, {30, 1, 120}}) {
		t.Errorf("the coin sent out came off the first lot at cost; the sale closed one at 100 and one at 120: %v", got)
	}
	byID := map[string]*Act{}
	for i := range acts {
		byID[acts[i].ID] = NormalizeActivity(&acts[i])
	}
	trades := BuildTrades(fifo.Closed, fifo.Open, nil, byID, NewSecurities(nil), noJournal())
	if len(trades) != 1 {
		t.Fatalf("trades: %s", jsonOf(trades))
	}
	if py.Round(trades[0].Pnl, 6) != 80 || trades[0].Qty != 2 {
		t.Errorf("trade: pnl %v qty %v", trades[0].Pnl, trades[0].Qty)
	}
	for _, f := range trades[0].Fills {
		if f.ID == "to" {
			t.Error("the transfer out is not a fill of the trade")
		}
	}
	fifo = MatchFIFO(ptrs([]store.Activity{fixtures.CryptoTransfer("to2", "ETH", 1, 200, "2026-01-10", true, "Ponzi")}))
	if len(fifo.Closed) != 0 || len(fifo.Open) != 0 || len(fifo.Unmatched) != 0 {
		t.Errorf("nothing held: closed %s open %s unmatched %s", jsonOf(fifo.Closed), jsonOf(fifo.Open), jsonOf(fifo.Unmatched))
	}
}

func TestCryptoBuySellAndReward(t *testing.T) {
	acts := []store.Activity{
		fixtures.Crypto("cb", "buy", "ETH", 2, 100, "2026-01-01", "Ponzi"),
		fixtures.Crypto("rw", "reward", "ETH", 1, 0, "2026-01-05", "Ponzi"),
		fixtures.Crypto("cs", "sell", "ETH", 3, 150, "2026-02-01", "Ponzi"),
	}
	norm := NormalizeActivities(acts)
	if norm[0].Kind != "Crypto" {
		t.Errorf("kind: %q", norm[0].Kind)
	}
	if !(norm[0].NetCashAmount < 0) {
		t.Errorf("net cash: %v", norm[0].NetCashAmount)
	}
	if !hasFlag(norm[1].Flags, "reward") {
		t.Errorf("flags: %v", norm[1].Flags)
	}
	fifo := MatchFIFO(norm)
	if len(fifo.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(fifo.Unmatched))
	}
	if len(fifo.Open) != 0 {
		t.Fatalf("open: %s", jsonOf(fifo.Open))
	}
	if len(fifo.Closed) != 2 {
		t.Fatalf("closed: %s", jsonOf(fifo.Closed))
	}
	almost(t, sumPnl(fifo.Closed), (150-100)*2+150*1, "pnl")
	for _, s := range fifo.Closed {
		if s.Kind != "Crypto" {
			t.Errorf("kind: %q", s.Kind)
		}
	}
}

func TestCryptoDustSellIsNotUnmatched(t *testing.T) {
	acts := []store.Activity{
		row(store.Activity{ID: "cb", ActivityType: "CRYPTO_BUY", RawType: "CRYPTO_BUY", Quantity: 1.0, UnitPrice: 100, NetCashAmount: 100, TransactionDate: "2026-01-01", Symbol: "DOGE", Currency: "CAD"}),
		row(store.Activity{ID: "cs", ActivityType: "CRYPTO_SELL", RawType: "CRYPTO_SELL", Quantity: 1.0000004, UnitPrice: 120, NetCashAmount: 120, TransactionDate: "2026-02-01", Symbol: "DOGE", Currency: "CAD"}),
	}
	fifo := MatchFIFO(NormalizeActivities(acts))
	if len(fifo.Unmatched) != 0 {
		t.Fatalf("unmatched: %s", jsonOf(fifo.Unmatched))
	}
	if len(fifo.Closed) != 1 {
		t.Fatalf("closed: %s", jsonOf(fifo.Closed))
	}
}

func TestPendingDistributionNoticeIsNotALot(t *testing.T) {
	acts := []store.Activity{
		buy("b", "RDDY", 100, 9, "2026-01-01"),
		row(store.Activity{ID: "stk", Category: "trade", ActivityType: "STKDIS", ActivitySubType: "BUY", RawType: "DIVIDEND", Quantity: 100, TransactionDate: "2026-02-01", Symbol: "RDDY", Currency: "CAD"}),
	}
	fifo := MatchFIFO(NormalizeActivities(acts))
	if len(fifo.Open) != 1 {
		t.Fatalf("open: %s", jsonOf(fifo.Open))
	}
	if fifo.Open[0].Qty != 100 {
		t.Errorf("qty: %v", fifo.Open[0].Qty)
	}
	if fifo.Open[0].Price != 9 {
		t.Errorf("price: %v", fifo.Open[0].Price)
	}
}

func TestUsdPnlUsesRatesOnFillDates(t *testing.T) {
	fx := map[string]float64{"2026-01-05": 1.40, "2026-02-05": 1.30}
	fifo := MatchFIFO(ptrs([]store.Activity{
		buy("b", "LUNR", 100, 10, "2026-01-05", fixtures.WithCurrency("USD")),
		sell("s", "LUNR", 100, 12, "2026-02-05", fixtures.WithCurrency("USD")),
	}))
	ApplyFX(fifo.Closed, fx)
	s := fifo.Closed[0]
	almost(t, s.Pnl, 200, "pnl")
	almost(t, s.PnlCad, 1200*1.30-1000*1.40, "pnl in CAD")
}

func TestRateWalksBackOverWeekendsAndFallsBack(t *testing.T) {
	fx := map[string]float64{"2026-01-02": 1.40}
	if rateOn(fx, "2026-01-04") != 1.40 {
		t.Errorf("rate on the Sunday: %v", rateOn(fx, "2026-01-04"))
	}
	if rateOn(fx, "2025-06-01") != FXFallback {
		t.Errorf("rate before the table: %v", rateOn(fx, "2025-06-01"))
	}
	if toCad(fx, 100, "CAD", "2026-01-04") != 100 {
		t.Errorf("CAD to CAD: %v", toCad(fx, 100, "CAD", "2026-01-04"))
	}
}
