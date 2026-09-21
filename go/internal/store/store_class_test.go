package store

import (
	"math"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func near(a, b float64) bool { return math.Abs(a-b) < 1e-7 }

func byCanonicalID(rows []Activity) map[string]Activity {
	out := map[string]Activity{}
	for _, a := range rows {
		out[a.CanonicalID] = a
	}
	return out
}

func TestStatusCarriesTheDataVersionSoThePageCanReload(t *testing.T) {
	s := temp(t)
	v0 := s.DataVersion()
	if v0 == "" {
		t.Fatal("no data version")
	}
	s.UpsertQuote("RDDY", Quote{Price: py.Ptr(4.75), FetchedAt: "2026-09-07T15:00:00Z"}, "")
	v1 := s.DataVersion()
	if v1 == v0 {
		t.Error("a quote does not move the data version")
	}
	s.UpsertDistributions("RDDY", []Distribution{{ExDate: "2026-09-30", PayDate: "2026-10-05", Amount: 0.2, Currency: "CAD"}}, "")
	if s.DataVersion() == v1 {
		t.Error("a distribution does not move the data version")
	}
}

func TestScaleStoredOptionUnitPriceMissingMultiplier(t *testing.T) {
	s := temp(t)
	s.ApplyWealthsimpleMapped([]Activity{
		{CanonicalID: "opt-cheap-1", OccurredAt: "2026-08-31T14:16:58Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", Symbol: "DRAM 19FEB27 1.00 CALL", Currency: "USD", Quantity: -10, UnitPrice: 11.25, NetCashAmount: 112.5, Category: "trade", Source: "wealthsimple", RawType: "OPTIONS_SELL"},
		{CanonicalID: "opt-ok-1", OccurredAt: "2026-08-31T14:17:58Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOOPEN", Symbol: "SOXL 19FEB27 20.00 CALL", Currency: "USD", Quantity: -10, UnitPrice: 13.3, NetCashAmount: 13300, Category: "trade", Source: "wealthsimple", RawType: "OPTIONS_SELL"},
		{CanonicalID: "share-ok-1", OccurredAt: "2026-08-31T14:18:58Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "Trade", ActivitySubType: "BUY", Symbol: "AAA", Currency: "CAD", Quantity: 10, UnitPrice: 10.0, NetCashAmount: -100, Category: "trade", Source: "wealthsimple", RawType: "DIY_BUY"},
	})
	s.DeleteMeta(OptionUnitPriceScaleMeta)
	if err := s.Ensure(); err != nil {
		t.Fatal(err)
	}
	byID := byCanonicalID(s.Snapshot(true).Activities)
	if !near(byID["opt-cheap-1"].UnitPrice, 0.1125) {
		t.Errorf("opt-cheap-1 unitPrice %v", byID["opt-cheap-1"].UnitPrice)
	}
	if !near(byID["opt-ok-1"].UnitPrice, 13.3) {
		t.Errorf("opt-ok-1 unitPrice %v", byID["opt-ok-1"].UnitPrice)
	}
	if !near(byID["share-ok-1"].UnitPrice, 10.0) {
		t.Errorf("share-ok-1 unitPrice %v", byID["share-ok-1"].UnitPrice)
	}
	if s.GetMeta(OptionUnitPriceScaleMeta) != "1" {
		t.Errorf("scale meta %q", s.GetMeta(OptionUnitPriceScaleMeta))
	}
	if err := s.Ensure(); err != nil {
		t.Fatal(err)
	}
	again := byCanonicalID(s.Snapshot(true).Activities)
	if !near(again["opt-cheap-1"].UnitPrice, 0.1125) || !near(again["opt-ok-1"].UnitPrice, 13.3) || !near(again["share-ok-1"].UnitPrice, 10.0) {
		t.Errorf("a second ensure rescaled: %+v", again)
	}
}

func TestRelabelStoredOptionsSell(t *testing.T) {
	s := temp(t)
	s.ApplyWealthsimpleMapped([]Activity{
		{CanonicalID: "opt-sell-1", OccurredAt: "2026-08-31T14:16:58Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "OPTIONS_SELL", ActivitySubType: "LIMIT_ORDER", Symbol: "QNC 19FEB27 3.00 CALL", Currency: "USD", Quantity: 35, UnitPrice: 0.3, NetCashAmount: 1050, Category: "other", Source: "wealthsimple", RawType: "OPTIONS_SELL"},
	})
	if err := s.Ensure(); err != nil {
		t.Fatal(err)
	}
	row, ok := byCanonicalID(s.Snapshot(true).Activities)["opt-sell-1"]
	if !ok {
		t.Fatal("row missing")
	}
	if row.ActivitySubType != "SELLTOOPEN" || row.Category != "trade" || row.Quantity != -35 || row.NetCashAmount != 1050 {
		t.Fatalf("%+v", row)
	}
}

func TestRelabelStoredOptionsMultilegAndExpiry(t *testing.T) {
	s := temp(t)
	s.ApplyWealthsimpleMapped([]Activity{
		{CanonicalID: "opt-ml-1", OccurredAt: "2026-08-31T14:16:58Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "OPTIONS_MULTILEG", ActivitySubType: "FILLED", Symbol: "LUNR 15JAN27 12.00 CALL", Currency: "USD", Quantity: 0, UnitPrice: 0, NetCashAmount: -128, Category: "other", Source: "wealthsimple", RawType: "OPTIONS_MULTILEG"},
		{CanonicalID: "opt-ml-credit", OccurredAt: "2026-08-31T14:16:59Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "OPTIONS_SELL", ActivitySubType: "SELLTOCLOSE", Symbol: "BBAI 21JAN28 10.00 CALL", Currency: "USD", Quantity: 0, UnitPrice: 0, NetCashAmount: 56, Category: "trade", Source: "wealthsimple", RawType: "OPTIONS_MULTILEG"},
		{CanonicalID: "opt-exp-1", OccurredAt: "2026-08-31T14:17:58Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "OPTIONS_SHORT_EXPIRY", ActivitySubType: "EXPIRED", Symbol: "LUNR 15JAN27 12.00 CALL", Currency: "USD", Quantity: 5, UnitPrice: 0, NetCashAmount: 0, Category: "other", Source: "wealthsimple", RawType: "OPTIONS_SHORT_EXPIRY"},
		{CanonicalID: "opt-long-exp", OccurredAt: "2026-08-31T14:17:59Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "EXPIR", ActivitySubType: "BUY", Symbol: "LUNR 22AUG25 12.00 CALL", Currency: "USD", Quantity: 4, UnitPrice: 0, NetCashAmount: 0, Category: "option_event", Source: "wealthsimple", RawType: "OPTIONS_EXPIRY"},
		{CanonicalID: "opt-asg-1", OccurredAt: "2026-08-31T14:18:58Z", TransactionDate: "2026-08-31", AccountID: "acct-1", AccountType: "Trading", ActivityType: "OPTIONS_ASSIGN", ActivitySubType: "ASSIGNED", Symbol: "LUNR 15JAN27 12.00 CALL", Currency: "USD", Quantity: -2, UnitPrice: 0, NetCashAmount: 0, Category: "other", Source: "wealthsimple", RawType: "OPTIONS_ASSIGN"},
		{CanonicalID: "opt-asg-strike", OccurredAt: "2025-03-07T21:00:00Z", TransactionDate: "2025-03-07", AccountID: "acct-1", AccountType: "Trading", ActivityType: "ASSIGN", ActivitySubType: "BUYTOCLOSE", Symbol: "ASTS 07MAR25 31.00 CALL", Currency: "USD", Quantity: 1, UnitPrice: 31, NetCashAmount: -3100, Category: "option_event", Source: "wealthsimple", RawType: "OPTIONS_ASSIGN"},
	})
	if err := s.Ensure(); err != nil {
		t.Fatal(err)
	}
	byID := byCanonicalID(s.Snapshot(true).Activities)
	ml := byID["opt-ml-1"]
	if ml.Category != "trade" || ml.ActivityType != "OPTIONS_BUY" || ml.ActivitySubType != "BUYTOCLOSE" || ml.Quantity != 0 || ml.NetCashAmount != -128 {
		t.Errorf("multileg debit: %+v", ml)
	}
	credit := byID["opt-ml-credit"]
	if credit.Category != "trade" || credit.ActivityType != "OPTIONS_SELL" || credit.ActivitySubType != "SELLTOOPEN" || credit.NetCashAmount != 56 {
		t.Errorf("multileg credit: %+v", credit)
	}
	exp := byID["opt-exp-1"]
	if exp.Category != "option_event" || exp.ActivityType != "EXPIR" || exp.ActivitySubType != "BUY" || exp.Quantity != 5 {
		t.Errorf("short expiry: %+v", exp)
	}
	longExp := byID["opt-long-exp"]
	if longExp.Category != "option_event" || longExp.ActivityType != "EXPIR" || longExp.ActivitySubType != "SELL" || longExp.Quantity != -4 {
		t.Errorf("long expiry: %+v", longExp)
	}
	asg := byID["opt-asg-1"]
	if asg.Category != "option_event" || asg.ActivityType != "ASSIGN" || asg.ActivitySubType != "BUYTOCLOSE" || asg.Quantity != 2 {
		t.Errorf("assign: %+v", asg)
	}
	strike := byID["opt-asg-strike"]
	if strike.UnitPrice != 0 || strike.ActivitySubType != "BUYTOCLOSE" {
		t.Errorf("assign at the strike: %+v", strike)
	}
}

func TestManualHasNoCanonicalID(t *testing.T) {
	s := temp(t)
	row := Activity{ID: py.UUID4(), OccurredAt: "2024-07-01", TransactionDate: "2024-07-01", SettlementDate: "2024-07-01", AccountID: "manual", BookID: "manual", AccountType: "Manual", ActivityType: "Trade", ActivitySubType: "BUY", Description: "Buy 3 ZZZ @ 12.5", Direction: "DEBIT", Symbol: "ZZZ", Name: "ZZZ", Currency: "CAD", Quantity: 3, UnitPrice: 12.5, NetCashAmount: -37.5, Category: "trade", Source: "manual"}
	result := s.MergeLocalRows([]Activity{row})
	if len(result.Activities) != 1 {
		t.Fatalf("%+v", result)
	}
	act := result.Activities[0]
	if act.ID == "" {
		t.Error("no id")
	}
	if LooksLikeHomemadeID(act.ID) {
		t.Errorf("homemade id %q", act.ID)
	}
	if act.CanonicalID != "" {
		t.Errorf("canonical id %q", act.CanonicalID)
	}
	stored := s.Snapshot(true).Activities[0]
	if stored.CanonicalID != "" || stored.Source != "manual" {
		t.Fatalf("%+v", stored)
	}
}

func TestDailyPathDoesNotPageWholeHistoryWhenRowsExist(t *testing.T) {
	s := temp(t)
	s.ApplyWealthsimpleMapped([]Activity{wsItem("ws-cid-aaa-001", "2024-06-15T13:45:22.123Z", 10, 10, -100)})
	start := s.IncrementalStartDate()
	if start == "" {
		t.Fatal("no start date")
	}
	if start[:10] != "2024-06-01" {
		t.Errorf("fourteen days before the newest stored row: %q", start)
	}
}

func TestDailyWindowReachesBackPastRowsFiledUnderALaterDay(t *testing.T) {
	s := temp(t)
	s.ApplyWealthsimpleMapped([]Activity{wsItem("ws-cid-card-0909", "2026-09-09T01:11:38.000Z", 10, 10, -100)})
	if got := s.IncrementalStartDate(); got != "2026-08-26" {
		t.Errorf("start date %q", got)
	}
}

func TestEmptyTableFullHistoryOmitsStartDate(t *testing.T) {
	s := temp(t)
	if s.ActivityCount() != 0 {
		t.Fatal("rows in a fresh store")
	}
	if got := s.IncrementalStartDate(); got != "" {
		t.Errorf("start date %q", got)
	}
}

func TestExistingRowsMakeDailySyncIncremental(t *testing.T) {
	s := temp(t)
	s.ApplyWealthsimpleMapped([]Activity{wsItem("ws-cid-aaa-001", "2024-06-15T13:45:22.123Z", 10, 10, -100)})
	if _, err := s.InsertLocal(Activity{TransactionDate: "2024-07-01", OccurredAt: "2024-07-01", AccountID: "manual", Symbol: "ZZZ", Quantity: 1, UnitPrice: 2, NetCashAmount: -2, ActivityType: "Trade", ActivitySubType: "BUY", Source: "manual"}); err != nil {
		t.Fatal(err)
	}
	if s.ActivityCount() != 2 {
		t.Fatalf("count %d", s.ActivityCount())
	}
	var ws, manual *Activity
	for _, a := range s.Snapshot(true).Activities {
		switch a.Source {
		case "wealthsimple":
			ws = &a
		case "manual":
			manual = &a
		}
	}
	if ws == nil || manual == nil {
		t.Fatal("both sources stored")
	}
	if ws.CanonicalID != "ws-cid-aaa-001" || LooksLikeHomemadeID(ws.ID) {
		t.Errorf("%+v", ws)
	}
	if manual.CanonicalID != "" {
		t.Errorf("%+v", manual)
	}
	if s.IncrementalStartDate() == "" {
		t.Error("no start date")
	}
}

func TestMarginRowsRoundTripAndClearWithTheSyncedData(t *testing.T) {
	s := temp(t)
	s.ReplaceMargin([]Margin{
		{AccountID: "acct-1", BuyingPower: py.Ptr(6817.33), Currency: "CAD"},
		{AccountID: "acct-2", Currency: "CAD", Unavailable: "UnavailableSecurities (2 securities)"},
		{AccountID: "", BuyingPower: py.Ptr(1.0)},
	})
	rows := s.Snapshot(false).Margin
	if len(rows) != 2 {
		t.Fatalf("%+v", rows)
	}
	if rows[0].AccountID != "acct-1" || rows[0].BuyingPower == nil || *rows[0].BuyingPower != 6817.33 || rows[0].Unavailable != "" {
		t.Errorf("%+v", rows[0])
	}
	if rows[1].AccountID != "acct-2" || rows[1].BuyingPower != nil || rows[1].Unavailable != "UnavailableSecurities (2 securities)" {
		t.Errorf("%+v", rows[1])
	}
	for _, r := range rows {
		if r.FetchedAt == "" {
			t.Errorf("no fetchedAt: %+v", r)
		}
	}
	v1 := s.DataVersion()
	s.ReplaceMargin([]Margin{{AccountID: "acct-1", BuyingPower: py.Ptr(6900.0), Currency: "CAD"}})
	if s.DataVersion() == v1 {
		t.Error("a new reading changes the model fingerprint")
	}
	s.ClearSyncedData(true, true)
	if got := s.Snapshot(false).Margin; len(got) != 0 {
		t.Errorf("%+v", got)
	}
}

func TestSnapshotIncludesSecurities(t *testing.T) {
	s := temp(t)
	s.UpsertSecurities([]Security{{ID: "sec-s-ch", Symbol: "CH", Name: "Charbone Corporation", PrimaryExchange: "TSX Venture Exchange", PrimaryMic: "XTSV", Currency: "CAD"}})
	secs := s.Snapshot(false).Securities
	if len(secs) != 1 {
		t.Fatalf("%+v", secs)
	}
	if secs[0].ID != "sec-s-ch" || secs[0].Name != "Charbone Corporation" || secs[0].PrimaryMic != "XTSV" {
		t.Fatalf("%+v", secs[0])
	}
}
