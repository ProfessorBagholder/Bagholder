package model

import (
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func roundTrips(acts []store.Activity, groups []store.TradeGroup, journal map[string]store.JournalEntry) []*Trade {
	norm := NormalizeActivities(acts)
	fifo := MatchFIFO(norm)
	ApplyFX(fifo.Closed, map[string]float64{})
	byID := map[string]*Act{}
	for _, a := range norm {
		byID[a.ID] = a
	}
	if journal == nil {
		journal = noJournal()
	}
	return BuildTrades(fifo.Closed, fifo.Open, groups, byID, NewSecurities(nil), journal)
}

func TestFlatToFlatTwiceIsTwoTrades(t *testing.T) {
	trades := roundTrips([]store.Activity{
		buy("b1", "AAA", 100, 10, "2026-01-01"),
		sell("s1", "AAA", 100, 12, "2026-01-10"),
		buy("b2", "AAA", 50, 11, "2026-02-01"),
		sell("s2", "AAA", 50, 9, "2026-02-10"),
	}, nil, nil)
	if len(trades) != 2 {
		t.Fatalf("trades: %s", jsonOf(trades))
	}
	ids := map[string]bool{}
	pnl := map[string]float64{}
	for _, tr := range trades {
		ids[tr.ID] = true
		pnl[tr.ID] = tr.Pnl
		if tr.Status != "closed" {
			t.Errorf("status: %q", tr.Status)
		}
	}
	if !reflect.DeepEqual(ids, map[string]bool{"rt:b1": true, "rt:b2": true}) {
		t.Errorf("ids: %v", ids)
	}
	almost(t, pnl["rt:b1"], 200, "first round trip")
	almost(t, pnl["rt:b2"], -100, "second round trip")
}

func TestScaleInAndOutIsOneTradeWithLegs(t *testing.T) {
	trades := roundTrips([]store.Activity{
		buy("b1", "AAA", 100, 10, "2026-01-01"),
		sell("s1", "AAA", 50, 12, "2026-01-10"),
		buy("b2", "AAA", 100, 11, "2026-01-15"),
		sell("s2", "AAA", 150, 13, "2026-02-01"),
	}, nil, nil)
	if len(trades) != 1 {
		t.Fatalf("trades: %s", jsonOf(trades))
	}
	tr := trades[0]
	if tr.ID != "rt:b1" {
		t.Errorf("id: %q", tr.ID)
	}
	if tr.Status != "closed" {
		t.Errorf("status: %q", tr.Status)
	}
	if tr.Qty != 200 {
		t.Errorf("qty: %v", tr.Qty)
	}
	if tr.LegCount != 3 {
		t.Errorf("legCount: %d", tr.LegCount)
	}
	if tr.EntryDate != "2026-01-01" {
		t.Errorf("entryDate: %q", tr.EntryDate)
	}
	if tr.ExitDate != "2026-02-01" {
		t.Errorf("exitDate: %q", tr.ExitDate)
	}
	almost(t, tr.Pnl, 50*2+50*3+100*2, "pnl")
	if len(tr.Fills) != 4 {
		t.Errorf("fills: %s", jsonOf(tr.Fills))
	}
	if tr.Opened.Fills != 2 {
		t.Errorf("opened fills: %d", tr.Opened.Fills)
	}
	if tr.Closed.Fills != 2 {
		t.Errorf("closed fills: %d", tr.Closed.Fills)
	}
	if tr.Side != "SELL" {
		t.Errorf("side: %q", tr.Side)
	}
}

func TestPartialExitIsAClosedTradeWithStableID(t *testing.T) {
	acts := []store.Activity{buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 40, 12, "2026-01-10")}
	trades := roundTrips(acts, nil, nil)
	if len(trades) != 1 {
		t.Fatalf("trades: %s", jsonOf(trades))
	}
	if trades[0].Status != "closed" {
		t.Errorf("status: %q", trades[0].Status)
	}
	if trades[0].ID != "rt:b1" {
		t.Errorf("id: %q", trades[0].ID)
	}
	if trades[0].Qty != 40 {
		t.Errorf("qty: %v", trades[0].Qty)
	}
	acts = append(acts, sell("s2", "AAA", 60, 15, "2026-02-01"))
	trades = roundTrips(acts, nil, nil)
	if len(trades) != 1 {
		t.Fatalf("trades: %s", jsonOf(trades))
	}
	if trades[0].Status != "closed" {
		t.Errorf("status: %q", trades[0].Status)
	}
	if trades[0].ID != "rt:b1" {
		t.Errorf("id: %q", trades[0].ID)
	}
	if trades[0].Qty != 100 {
		t.Errorf("qty: %v", trades[0].Qty)
	}
}

func TestSavedGroupOverridesRoundTrip(t *testing.T) {
	acts := []store.Activity{
		buy("b1", "AAA", 100, 10, "2026-01-01"),
		sell("s1", "AAA", 100, 12, "2026-01-10"),
		buy("b2", "AAA", 50, 11, "2026-02-01"),
		sell("s2", "AAA", 50, 9, "2026-02-10"),
	}
	key1 := "b1|s1|" + fmt8(100)
	key2 := "b2|s2|" + fmt8(50)
	trades := roundTrips(acts, []store.TradeGroup{{ID: "g_manual", Locked: true, Members: []string{key1, key2}}}, nil)
	if len(trades) != 1 {
		t.Fatalf("trades: %s", jsonOf(trades))
	}
	if trades[0].ID != "g_manual" {
		t.Errorf("id: %q", trades[0].ID)
	}
	if !trades[0].Locked {
		t.Error("locked")
	}
	if trades[0].LegCount != 2 {
		t.Errorf("legCount: %d", trades[0].LegCount)
	}
}

func TestPositionNotesCarryOverToTheClosedTrade(t *testing.T) {
	snapshot := snap(buy("b1", "AAA", 100, 10, "2026-01-01"))
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-02-01", nil)
	pid := base.Positions[0].ID
	if pid != "rt:b1" {
		t.Fatalf("position id: %q", pid)
	}
	journal := map[string]store.JournalEntry{pid: {Thesis: "holding for the catalyst", Tags: []string{"core"}, Grade: ""}}
	base = BuildBase(snapshot, mkt(nil, nil), journal, "2026-02-01", nil)
	if base.Positions[0].Thesis != "holding for the catalyst" {
		t.Errorf("thesis: %q", base.Positions[0].Thesis)
	}
	snapshot.Activities = append(snapshot.Activities, sell("s1", "AAA", 100, 12, "2026-03-01"))
	base = BuildBase(snapshot, mkt(nil, nil), journal, "2026-04-01", nil)
	if len(base.Positions) != 0 {
		t.Errorf("positions: %s", jsonOf(base.Positions))
	}
	if base.Trades[0].ID != "rt:b1" {
		t.Errorf("trade id: %q", base.Trades[0].ID)
	}
	if base.Trades[0].Thesis != "holding for the catalyst" {
		t.Errorf("thesis: %q", base.Trades[0].Thesis)
	}
	if !reflect.DeepEqual(base.Trades[0].Tags, []string{"core"}) {
		t.Errorf("tags: %v", base.Trades[0].Tags)
	}
}

func TestJournalAttachesToTrade(t *testing.T) {
	trades := roundTrips(
		[]store.Activity{buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")},
		nil,
		map[string]store.JournalEntry{"rt:b1": {Thesis: "breakout", Tags: []string{"momo"}, Grade: "A"}},
	)
	if trades[0].Grade != "A" {
		t.Errorf("grade: %q", trades[0].Grade)
	}
	if !reflect.DeepEqual(trades[0].Tags, []string{"momo"}) {
		t.Errorf("tags: %v", trades[0].Tags)
	}
	if trades[0].Thesis != "breakout" {
		t.Errorf("thesis: %q", trades[0].Thesis)
	}
}

func fillSubsByID(fills []Fill) map[string]string {
	out := map[string]string{}
	for _, f := range fills {
		out[f.ID] = f.Sub
	}
	return out
}

func TestFillLabelsReflectWhatTheFillDid(t *testing.T) {
	trades := roundTrips([]store.Activity{
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -2, UnitPrice: 3, NetCashAmount: 600, TransactionDate: "2026-01-01", Symbol: "ZZZ 21AUG26 10.00 CALL"}),
		row(store.Activity{ID: "buy", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 2, UnitPrice: 1, NetCashAmount: -200, TransactionDate: "2026-02-01", Symbol: "ZZZ 21AUG26 10.00 CALL"}),
	}, nil, nil)
	subs := fillSubsByID(trades[0].Fills)
	if !reflect.DeepEqual(subs, map[string]string{"sto": "SELL TO OPEN", "buy": "BUY TO CLOSE"}) {
		t.Errorf("subs: %v", subs)
	}
	trades = roundTrips([]store.Activity{buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")}, nil, nil)
	subs = fillSubsByID(trades[0].Fills)
	if !reflect.DeepEqual(subs, map[string]string{"b1": "BUY", "s1": "SELL"}) {
		t.Errorf("subs: %v", subs)
	}
}

func TestShortRoundTripIsCover(t *testing.T) {
	trades := roundTrips([]store.Activity{
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -2, UnitPrice: 3, NetCashAmount: 600, TransactionDate: "2026-01-01", Symbol: "ZZZ 21AUG26 10.00 CALL"}),
		row(store.Activity{ID: "btc", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOCLOSE", RawType: "OPTIONS_BUY", Quantity: 2, UnitPrice: 1, NetCashAmount: -200, TransactionDate: "2026-02-01", Symbol: "ZZZ 21AUG26 10.00 CALL"}),
	}, nil, nil)
	if len(trades) != 1 {
		t.Fatalf("trades: %s", jsonOf(trades))
	}
	if trades[0].Side != "COVER" {
		t.Errorf("side: %q", trades[0].Side)
	}
	if trades[0].Kind != "Options" {
		t.Errorf("kind: %q", trades[0].Kind)
	}
	if trades[0].Mult != 100 {
		t.Errorf("mult: %v", trades[0].Mult)
	}
	almost(t, trades[0].Pnl, 400, "pnl")
	almost(t, deref(trades[0].PnlPct), 400.0/600, "pnl pct")
}

func TestOptionExpiryParse(t *testing.T) {
	if got := optionExpiry("LUNR 29AUG25 11.50 CALL"); got != "2025-08-29" {
		t.Errorf("got %q", got)
	}
	if got := optionExpiry("BBAI 02JAN26 5.50 PUT"); got != "2026-01-02" {
		t.Errorf("got %q", got)
	}
	if got := optionExpiry("AAPL"); got != "" {
		t.Errorf("got %q", got)
	}
}

func TestOpenOptionPastExpiryIsClosedAtZero(t *testing.T) {
	snapshot := snap(
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -2, UnitPrice: 0.3, NetCashAmount: 60, TransactionDate: "2025-12-05", Symbol: "BBAI 02JAN26 5.50 PUT"}),
		row(store.Activity{ID: "bto", Category: "trade", ActivityType: "OPTIONS_BUY", ActivitySubType: "BUYTOOPEN", RawType: "OPTIONS_BUY", Quantity: 1, UnitPrice: 1.0, NetCashAmount: -100, TransactionDate: "2026-01-05", Symbol: "ZZZ 17JUL26 10.00 CALL"}),
	)
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-03-01", nil)
	if len(base.OpenLots) != 1 {
		t.Fatalf("open lots: %s", jsonOf(base.OpenLots))
	}
	if base.OpenLots[0].Symbol != "ZZZ 17JUL26 10.00 CALL" {
		t.Errorf("open symbol: %q", base.OpenLots[0].Symbol)
	}
	if len(base.Trades) != 1 {
		t.Fatalf("trades: %s", jsonOf(base.Trades))
	}
	tr := base.Trades[0]
	if tr.ExitDate != "2026-01-02" {
		t.Errorf("exitDate: %q", tr.ExitDate)
	}
	if tr.Exit != 0 {
		t.Errorf("exit: %v", tr.Exit)
	}
	almost(t, tr.Pnl, 60, "pnl")
	if !hasFlag(tr.Flags, "assumed-expiry") {
		t.Errorf("flags: %v", tr.Flags)
	}
	if tr.Status != "closed" {
		t.Errorf("status: %q", tr.Status)
	}
}

func TestAssignedCallDeliversTheShares(t *testing.T) {
	snapshot := snap(
		buy("b1", "ASTS", 300, 25.0, "2025-01-10", fixtures.WithCurrency("USD"), fixtures.WithSecurity("sec-s-asts")),
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -3, UnitPrice: 1.5, NetCashAmount: 450, TransactionDate: "2025-02-10", Symbol: "ASTS 07MAR25 31.00 CALL", SecurityID: "sec-o-asts"}),
		row(store.Activity{ID: "asg", Category: "option_event", ActivityType: "ASSIGN", ActivitySubType: "BUYTOCLOSE", RawType: "OPTIONS_ASSIGN", Quantity: 3, UnitPrice: 0, NetCashAmount: 9300, TransactionDate: "2025-03-07", Symbol: "ASTS 07MAR25 31.00 CALL", SecurityID: "sec-o-asts"}),
	)
	snapshot.Securities = []store.Security{
		{ID: "sec-o-asts", Symbol: "ASTS", UnderlyingID: "sec-s-asts"},
		{ID: "sec-s-asts", Symbol: "ASTS", Name: "AST SpaceMobile", PrimaryExchange: "NASDAQ"},
	}
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-09-06", nil)
	if len(base.OpenLots) != 0 {
		t.Fatalf("open lots: %s", jsonOf(base.OpenLots))
	}
	bySym := map[string]*Trade{}
	for _, tr := range base.Trades {
		bySym[tr.Symbol] = tr
	}
	shares := bySym["ASTS"]
	if shares == nil {
		t.Fatalf("no ASTS trade: %s", jsonOf(base.Trades))
	}
	if shares.Qty != 300 {
		t.Errorf("qty: %v", shares.Qty)
	}
	if shares.Exit != 31.0 {
		t.Errorf("exit: %v", shares.Exit)
	}
	if shares.ExitDate != "2025-03-07" {
		t.Errorf("exitDate: %q", shares.ExitDate)
	}
	almost(t, shares.Pnl, (31-25)*300, "pnl")
	if !hasFlag(shares.Flags, "assignment") {
		t.Errorf("flags: %v", shares.Flags)
	}
	if shares.Name != "AST SpaceMobile" {
		t.Errorf("name: %q", shares.Name)
	}
	almost(t, bySym["ASTS 07MAR25 31.00 CALL"].Pnl, 450, "option pnl")
}

func TestAssignedPutBuysTheShares(t *testing.T) {
	snapshot := snap(
		row(store.Activity{ID: "sto", Category: "trade", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", RawType: "OPTIONS_SELL", Quantity: -1, UnitPrice: 0.5, NetCashAmount: 50, TransactionDate: "2025-11-10", Symbol: "BBAI 05DEC25 5.00 PUT"}),
		row(store.Activity{ID: "asg", Category: "option_event", ActivityType: "ASSIGN", ActivitySubType: "BUYTOCLOSE", RawType: "OPTIONS_ASSIGN", Quantity: 1, UnitPrice: 0, NetCashAmount: -500, TransactionDate: "2025-12-05", Symbol: "BBAI 05DEC25 5.00 PUT"}),
	)
	base := BuildBase(snapshot, mkt(nil, nil), noJournal(), "2026-01-01", nil)
	got := [][]any{}
	for _, l := range base.OpenLots {
		got = append(got, []any{l.Symbol, l.Qty, l.Price})
	}
	if !sameJSON(got, [][]any{{"BBAI", 100.0, 5.0}}) {
		t.Fatalf("open lots: %s", jsonOf(got))
	}
	if !hasFlag(base.OpenLots[0].Flags, "assignment") {
		t.Errorf("flags: %v", base.OpenLots[0].Flags)
	}
}

func TestGroupIDMatchesLedgerHTML(t *testing.T) {
	if GroupIDForKeys([]string{"b|s|100.00000000"}) != GroupIDForKeys([]string{"b|s|100.00000000"}) {
		t.Error("the id is not stable")
	}
	if id := GroupIDForKeys([]string{"a", "b"}); id[len(id)-2:] != "_2" {
		t.Errorf("id %q does not end with _2", id)
	}
	if GroupIDForKeys([]string{"a", "b"}) != GroupIDForKeys([]string{"b", "a"}) {
		t.Error("the id depends on the order of the keys")
	}
}

func TestLegacyNoteLandsOnRoundTrip(t *testing.T) {
	acts := NormalizeActivities([]store.Activity{buy("b1", "AAA", 100, 10, "2026-01-01"), sell("s1", "AAA", 100, 12, "2026-01-10")})
	fifo := MatchFIFO(acts)
	key := sliceMemberKey(fifo.Closed[0])
	legacyID := GroupIDForKeys([]string{key})
	journal := MigrateLegacyNotes(fifo.Closed, nil, map[string]store.TradeNote{legacyID: {Thesis: "why", Tag: "a, b", Grade: "C"}})
	want := map[string]store.JournalEntry{"rt:b1": {Thesis: "why", Tags: []string{"a", "b"}, Grade: "C"}}
	if !reflect.DeepEqual(journal, want) {
		t.Errorf("journal: %s, want %s", jsonOf(journal), jsonOf(want))
	}
}
