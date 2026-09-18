package store

import (
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

func temp(t *testing.T) *Store {
	t.Helper()
	s, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { s.Close() })
	return s
}

func wsItem(cid string, occurred string, qty, px, cash float64) Activity {
	return Activity{CanonicalID: cid, OccurredAt: occurred, TransactionDate: occurred[:10], SettlementDate: occurred[:10], AccountID: "acct-1", BookID: "acct-1", FifoID: "acct-1",
		AccountType: "Trading", ActivityType: "Trade", ActivitySubType: "BUY", Description: "Buy 10 AAA @ 10", Direction: "DEBIT", Symbol: "AAA", Name: "AAA", Currency: "CAD",
		Quantity: qty, UnitPrice: px, Commission: 0, NetCashAmount: cash, Category: "trade", Source: "wealthsimple", RawType: "DIY_BUY"}
}

func TestInsertIfNewByCanonicalID(t *testing.T) {
	s := temp(t)
	row := wsItem("ws-cid-aaa-001", "2024-06-15T13:45:22.123Z", 10, 10, -100)
	first := s.ApplyWealthsimpleMapped([]Activity{row})
	if first.Inserted != 1 || s.ActivityCount() != 1 {
		t.Fatalf("first: %+v count %d", first, s.ActivityCount())
	}
	again := s.ApplyWealthsimpleMapped([]Activity{row})
	if again.Inserted != 0 || again.Skipped != 1 || s.ActivityCount() != 1 {
		t.Fatalf("again: %+v", again)
	}
	stored := s.Snapshot(true).Activities[0]
	if stored.OccurredAt != "2024-06-15T13:45:22.123Z" || stored.TransactionDate != "2024-06-15" {
		t.Errorf("occurredAt kept whole: %+v", stored)
	}
}

func TestARowWealthsimpleRevisesReplacesTheStoredCopy(t *testing.T) {
	s := temp(t)
	original := wsItem("ws-cid-aaa-001", "2024-06-15T13:45:22.123Z", 10, 10, -100)
	s.ApplyWealthsimpleMapped([]Activity{original})
	storedID := s.Snapshot(true).Activities[0].ID
	changed := wsItem("ws-cid-aaa-001", "2024-06-15T13:45:22.123Z", 10, 99.9, -999)
	changed.Description = "revised by Wealthsimple"
	r := s.ApplyWealthsimpleMapped([]Activity{changed})
	if r.Inserted != 0 || r.Revised != 1 || r.Skipped != 0 {
		t.Fatalf("revise: %+v", r)
	}
	rows := s.Snapshot(true).Activities
	if len(rows) != 1 || rows[0].CanonicalID != "ws-cid-aaa-001" || rows[0].ID != storedID || rows[0].Description != "revised by Wealthsimple" || rows[0].NetCashAmount != -999 {
		t.Fatalf("stored: %+v", rows)
	}
	same := s.ApplyWealthsimpleMapped([]Activity{changed})
	if same.Revised != 0 || same.Skipped != 1 {
		t.Fatalf("unchanged row rewritten: %+v", same)
	}
}

func TestPlaceholderDividendBecomesThePaidDividend(t *testing.T) {
	s := temp(t)
	placeholder := wsItem("div_E002026619494", "2026-08-31T04:00:00.000Z", 4000, 0, 0)
	s.ApplyWealthsimpleMapped([]Activity{placeholder})
	paid := wsItem("div_E002026619494", "2026-09-08T14:02:11.000Z", 4000, 0.255, 1020)
	r := s.ApplyWealthsimpleMapped([]Activity{paid})
	rows := s.Snapshot(true).Activities
	if r.Revised != 1 || len(rows) != 1 || rows[0].TransactionDate != "2026-09-08" || rows[0].NetCashAmount != 1020 {
		t.Fatalf("%+v %+v", r, rows)
	}
}

func TestLinkSingleManualMatchStampsCanonicalID(t *testing.T) {
	s := temp(t)
	manual := Activity{ID: "x", OccurredAt: "2024-06-15", TransactionDate: "2024-06-15", AccountID: "acct-1", ActivityType: "Trade", ActivitySubType: "BUY", Symbol: "AAA", Currency: "CAD", Quantity: 10, UnitPrice: 10, NetCashAmount: -100, Category: "trade", Source: "manual"}
	s.MergeLocalRows([]Activity{manual})
	if s.ActivityCount() != 1 {
		t.Fatal("manual row not stored")
	}
	s.ApplyWealthsimpleMapped([]Activity{wsItem("ws-cid-aaa-001", "2024-06-15T13:45:22.123Z", 10, 10, -100)})
	rows := s.Snapshot(true).Activities
	if len(rows) != 1 || rows[0].CanonicalID != "ws-cid-aaa-001" || rows[0].Source != "manual" {
		t.Fatalf("link: %+v", rows)
	}
}

func TestSecondSyncStampsSecurityIDAndTakesTheRevision(t *testing.T) {
	s := temp(t)
	row := wsItem("ws-cid-aaa-001", "2024-06-15T13:45:22.123Z", 10, 10, -100)
	s.ApplyWealthsimpleMapped([]Activity{row})
	later := row
	later.SecurityID, later.Quantity, later.NetCashAmount = "sec-s-later", 999, 1
	again := s.ApplyWealthsimpleMapped([]Activity{later})
	if again.Inserted != 0 || again.Revised != 1 || again.Skipped != 0 {
		t.Fatalf("%+v", again)
	}
	got := s.Snapshot(true).Activities[0]
	if got.SecurityID != "sec-s-later" || got.Quantity != 999 || got.NetCashAmount != 1 {
		t.Fatalf("%+v", got)
	}
	unchanged := later
	unchanged.SecurityID = "sec-s-other"
	again = s.ApplyWealthsimpleMapped([]Activity{unchanged})
	if again.Revised != 0 || again.Skipped != 1 || s.Snapshot(true).Activities[0].SecurityID != "sec-s-later" {
		t.Fatalf("a security id already stored is kept: %+v", again)
	}
}

func TestLocalRowsNeverKeepACanonicalID(t *testing.T) {
	s := temp(t)
	saved, err := s.InsertLocal(Activity{TransactionDate: "2024-07-02", OccurredAt: "2024-07-02", AccountID: "acct-1", Symbol: "ZZZ", Quantity: 4, UnitPrice: 8, NetCashAmount: -32, ActivityType: "Trade", ActivitySubType: "BUY", Source: "csv", CanonicalID: "do-not-keep-this"})
	if err != nil || saved.CanonicalID != "" || saved.Source != "csv" || LooksLikeHomemadeID(saved.ID) {
		t.Fatalf("%+v %v", saved, err)
	}
	if saved.OccurredAt != "2024-07-02" {
		t.Errorf("a date-only source stays date-only: %q", saved.OccurredAt)
	}
}

func TestMergeLocalRowsDropsDuplicates(t *testing.T) {
	s := temp(t)
	row := Activity{TransactionDate: "2026-01-01", Symbol: "AAA", Category: "trade", ActivitySubType: "BUY", Quantity: 1, UnitPrice: 2, NetCashAmount: -2, Currency: "CAD", Source: "csv"}
	r := s.MergeLocalRows([]Activity{row, row})
	if r.Added != 2 || r.Duplicates != 0 {
		t.Fatalf("two identical rows in one file are two rows: %+v", r)
	}
	r = s.MergeLocalRows([]Activity{row, row, row})
	if r.Added != 1 || r.Duplicates != 2 {
		t.Fatalf("a third copy is new, the two stored ones are duplicates: %+v", r)
	}
}

func TestActivityPullDueWeekdaysAt2pmMountain(t *testing.T) {
	s := temp(t)
	mt := ActivityPullTZ
	at := func(y int, m time.Month, d, h, mi int) time.Time { return time.Date(y, m, d, h, mi, 0, 0, mt) }
	if s.ActivityPullDue(at(2026, 8, 31, 13, 59)) {
		t.Error("before two")
	}
	if !s.ActivityPullDue(at(2026, 8, 31, 14, 0)) {
		t.Error("at two")
	}
	if s.ActivityPullDue(at(2026, 8, 29, 15, 0)) {
		t.Error("saturday")
	}
	s.MarkActivityPulled(py.Stamp(at(2026, 8, 31, 14, 5)))
	if s.ActivityPullDue(at(2026, 8, 31, 15, 0)) {
		t.Error("pulled today already")
	}
	if !s.ActivityPullDue(at(2026, 9, 1, 15, 0)) {
		t.Error("the next day is due again")
	}
}

func TestIncrementalStartDate(t *testing.T) {
	s := temp(t)
	if s.IncrementalStartDate() != "" {
		t.Error("empty table has no start")
	}
	s.ApplyWealthsimpleMapped([]Activity{wsItem("c1", "2026-09-10T13:00:00.000Z", 1, 1, -1)})
	if got := s.IncrementalStartDate(); got != "2026-08-27" {
		t.Errorf("fourteen days before the newest row: %q", got)
	}
}

func TestTradeGroupsRoundtrip(t *testing.T) {
	s := temp(t)
	groups := []any{
		map[string]any{"id": "g_one", "locked": true, "members": []any{"a|b|1.00000000", "c|d|2.00000000"}},
		map[string]any{"id": "g_one", "locked": false, "members": []any{"dup"}},
		map[string]any{"id": "", "members": []any{"x"}},
		map[string]any{"id": "g_empty", "members": []any{}},
		map[string]any{"id": "g_two", "locked": 1.0, "members": []any{"x", "x", "y"}},
	}
	saved := s.SaveTradeGroups(groups)
	if len(saved) != 2 || saved[0].ID != "g_one" || saved[1].ID != "g_two" || len(saved[1].Members) != 2 || !saved[0].Locked || !saved[1].Locked {
		t.Fatalf("%+v", saved)
	}
	if got := s.TradeGroups(); len(got) != 2 || got[0].Members[1] != "c|d|2.00000000" {
		t.Fatalf("%+v", got)
	}
	s.SetMeta("trade_groups", "{}")
	if len(s.TradeGroups()) != 0 || len(s.SaveTradeGroups(nil)) != 0 {
		t.Error("a non-list is rejected")
	}
}

func TestTradeNotesRoundtrip(t *testing.T) {
	s := temp(t)
	saved := s.SaveTradeNotes(map[string]any{
		"g_one":   map[string]any{"thesis": "scale in", "tag": "hold", "grade": "A"},
		"g_empty": map[string]any{"thesis": "", "tag": "", "grade": ""},
		"g_bad":   map[string]any{"thesis": "x", "tag": "y", "grade": "Z"},
		"":        map[string]any{"thesis": "nope"},
	})
	if saved["g_one"].Grade != "A" || saved["g_one"].Tag != "hold" || saved["g_bad"].Grade != "" || saved["g_bad"].Thesis != "x" {
		t.Fatalf("%+v", saved)
	}
	if _, ok := saved["g_empty"]; ok {
		t.Error("empty note kept")
	}
	if got := s.TradeNotes(); got["g_one"] != saved["g_one"] {
		t.Error("roundtrip")
	}
}

func TestFXAndBenchmarkRoundtrip(t *testing.T) {
	s := temp(t)
	if s.FXLastDate() != "" {
		t.Error("fresh store has a date")
	}
	if n := s.UpsertFXRates(map[string]float64{"2026-01-02": 1.4, "bad": 1, "2026-01-03": 0}); n != 1 {
		t.Errorf("wrote %d", n)
	}
	s.UpsertFXRates(map[string]float64{"2026-01-02": 9.9})
	if fx := s.FXRates(); len(fx) != 1 || fx["2026-01-02"] != 1.4 {
		t.Errorf("a day's rate is written once: %v", fx)
	}
	if s.FXLastDate() != "2026-01-02" {
		t.Error("last date")
	}
	s.UpsertBenchmarkPrices(map[string]float64{"2026-01-02": 5000, "2026-01-05": 5100}, BenchmarkSymbol)
	if s.BenchmarkLastDate(BenchmarkSymbol) != "2026-01-05" || s.MarketData().Benchmark["2026-01-05"] != 5100 {
		t.Error("benchmark")
	}
}

func TestJournalRoundtripAndVersion(t *testing.T) {
	s := temp(t)
	v0 := s.DataVersion()
	s.SaveJournalEntry("rt:x", map[string]any{"thesis": "t", "tags": []any{"a", "a", " b "}, "grade": "z"})
	j := s.Journal()
	if e := j["rt:x"]; e.Thesis != "t" || len(e.Tags) != 2 || e.Tags[1] != "b" || e.Grade != "" {
		t.Fatalf("%+v", j)
	}
	if v0 == s.DataVersion() {
		t.Error("version unchanged")
	}
	s.SaveJournalEntry("rt:x", map[string]any{"thesis": "", "tags": []any{}, "grade": ""})
	if len(s.Journal()) != 0 {
		t.Error("empty entry deletes")
	}
}

func TestClearSyncedDataKeepsJournalAndMarketByDefault(t *testing.T) {
	s := temp(t)
	s.MergeLocalRows([]Activity{{ID: "b1", TransactionDate: "2026-01-01", Symbol: "AAA", Category: "trade", ActivityType: "Trade", ActivitySubType: "BUY", Quantity: 10, UnitPrice: 1, NetCashAmount: -10, Currency: "CAD", Source: "csv"}})
	s.ReplaceAccounts([]Account{{ID: "acct-1", Nickname: "Trading"}})
	s.UpsertNav([]NavPoint{{Date: "2026-01-05", Equity: 20, NetDeposits: py.Ptr(10)}})
	s.SetMeta("synced_at", "2026-01-05T00:00:00Z")
	s.UpsertFXRates(map[string]float64{"2026-01-05": 1.4})
	s.SaveJournalEntry("rt:b1", map[string]any{"grade": "A"})
	before := s.DataSummary()
	if before["activities"] != 1 || before["accounts"] != 1 || before["navDays"] != 1 || before["journal"] != 1 {
		t.Fatalf("%v", before)
	}
	after := s.ClearSyncedData(true, true)
	if after["activities"] != 0 || after["accounts"] != 0 || after["navDays"] != 0 || after["syncedAt"] != "" || after["journal"] != 1 || after["fxDays"] != 1 {
		t.Fatalf("%v", after)
	}
	s.UpsertPriceHistory("AAA", []DailyBar{{Date: "2026-01-05", Close: 2}}, "yahoo")
	s.UpsertPriceBars("AAA", "1h", []Bar{{Time: 1767600000, Close: 2}}, "yahoo")
	s.SetMeta("market_attempt_at", "2026-01-05T00:00:00Z")
	s.SetMeta("tmx_form:AAA", "AAA")
	s.SetMeta("yahoo_miss:AAA.V", "1")
	after = s.ClearSyncedData(false, false)
	if after["journal"] != 0 || after["fxDays"] != 0 || len(s.PriceHistory("AAA", "", "")) != 0 || len(s.PriceBars("AAA", "1h", 0, 1<<40)) != 0 || s.GetMeta("market_attempt_at") != "" || s.GetMeta("tmx_form:AAA") != "" || s.GetMeta("yahoo_miss:AAA.V") != "" {
		t.Fatalf("%v", after)
	}
	if s.GetMeta("schema_version") != "13" {
		t.Error("schema version")
	}
}

func TestVersions(t *testing.T) {
	s := temp(t)
	s.UpsertQuote("AAA", Quote{Price: py.Ptr(10)}, "tmx")
	fullBefore, coreBefore := s.Versions()
	s.UpsertQuote("AAA", Quote{Price: py.Ptr(11)}, "tmx")
	fullAfter, coreAfter := s.Versions()
	if fullBefore == fullAfter {
		t.Error("the page is told the price moved")
	}
	if coreBefore != coreAfter {
		t.Error("nothing else did, so the match is kept")
	}
	s.InsertLocal(Activity{ID: "r1", TransactionDate: "2026-03-03", Symbol: "BBB", Category: "trade", ActivitySubType: "BUY", Quantity: 1, UnitPrice: 3, NetCashAmount: -3, Currency: "CAD"})
	full2, core2 := s.Versions()
	if full2 == fullAfter || core2 == coreAfter {
		t.Error("a row moves both")
	}
}

func TestQuoteUpsertKeepsDividendFields(t *testing.T) {
	s := temp(t)
	s.UpsertQuote("AAA", Quote{Price: py.Ptr(10), DividendAmount: py.Ptr(0.5), DividendFrequency: "monthly", ExDividendDate: "2026-09-30"}, "tmx")
	s.UpsertQuote("AAA", Quote{Price: py.Ptr(11)}, "tmx")
	q := s.Quotes()["AAA"]
	if *q.Price != 11 || *q.DividendAmount != 0.5 || q.DividendFrequency != "monthly" || q.ExDividendDate != "2026-09-30" {
		t.Fatalf("%+v", q)
	}
}

func TestPriceHistoryNewestDayMayBeReplaced(t *testing.T) {
	s := temp(t)
	s.UpsertPriceHistory("AAA", []DailyBar{{Date: "2026-01-05", Close: 2}, {Date: "2026-01-06", Close: 3}}, "tmx")
	s.UpsertPriceHistory("AAA", []DailyBar{{Date: "2026-01-05", Close: 9}, {Date: "2026-01-06", Close: 4}}, "tmx")
	bars := s.PriceHistory("AAA", "", "")
	if bars[0].Close != 2 || bars[1].Close != 4 {
		t.Fatalf("%+v", bars)
	}
	s.MarkHistoryFetched("AAA", "2026-01-01", "2026-01-06T00:00:00Z")
	s.MarkHistoryFetched("AAA", "2026-01-03", "2026-01-07T00:00:00Z")
	if f := s.HistoryFetch("AAA"); f.Start != "2026-01-01" || f.FetchedAt != "2026-01-07T00:00:00Z" {
		t.Fatalf("%+v", f)
	}
}

func TestNotifications(t *testing.T) {
	s := temp(t)
	row := s.AddNotification("fills", "k1", "Order filled · QNC", "Bought 5", nil, false)
	if row == nil || row.ID != 1 || row.SeenAt != "" {
		t.Fatalf("%+v", row)
	}
	if s.AddNotification("fills", "k1", "again", "", nil, false) != nil {
		t.Error("a key is told once")
	}
	seen := s.AddNotification("test", "k2", "t", "b", map[string]any{"symbol": "QNC"}, true)
	if seen.SeenAt == "" || seen.Extra["symbol"] != "QNC" {
		t.Fatalf("%+v", seen)
	}
	if n := s.UnreadNotifications(); n != 2 {
		t.Errorf("unread %d", n)
	}
	if s.MarkNotificationsSeen([]int64{1}) != 1 || s.MarkNotificationsSeen([]int64{1}) != 0 {
		t.Error("seen once")
	}
	if s.MarkNotificationsRead(nil, true) != 2 || s.UnreadNotifications() != 0 {
		t.Error("read all")
	}
	if got := s.ListNotifications(0, "", false, 50, true); len(got) != 2 || got[0].ID <= got[1].ID {
		t.Errorf("newest first: %+v", got)
	}
	for i := 0; i < NotificationsKept+5; i++ {
		s.AddNotification("x", py.UUID4(), "t", "", nil, false)
	}
	if got := s.ListNotifications(0, "", false, 1000, false); len(got) != NotificationsKept {
		t.Errorf("kept %d", len(got))
	}
	if s.ClearNotifications() != int64(NotificationsKept) {
		t.Error("clear")
	}
}

func TestOrdersAndBrackets(t *testing.T) {
	s := temp(t)
	s.InsertOrder(Order{ID: "order-1", AccountID: "acct-1", Account: "Trading", SecurityID: "sec-1", Symbol: "QNC", Currency: "CAD", Side: "BUY", Type: "LIMIT", Quantity: 5, LimitPrice: py.Ptr(1.75), Tif: "DAY", StopLoss: &StopLoss{Kind: "stop", Price: py.Ptr(1.6), TrailUnit: "pct"}, Status: "sent", Request: map[string]any{"quantity": 5.0, "externalId": "order-1"}})
	o := s.GetOrder("order-1")
	if o == nil || o.StopLoss == nil || *o.StopLoss.Price != 1.6 || o.Source != "bagholder" || o.Role != "entry" || o.Request["externalId"] != "order-1" {
		t.Fatalf("%+v", o)
	}
	s.UpdateOrder("order-1", OrderPatch{"status": "filled", "filledQty": 5.0, "avgFill": 1.74})
	o = s.GetOrder("order-1")
	if o.Status != "filled" || *o.FilledQty != 5 || *o.AvgFill != 1.74 {
		t.Fatalf("%+v", o)
	}
	if !s.MarkOrderFillBooked("order-1", 5) || s.MarkOrderFillBooked("order-1", 5) {
		t.Error("booked once")
	}
	s.InsertBracket(Bracket{ID: "bracket-1", OrderID: "order-1", AccountID: "acct-1", SecurityID: "sec-1", Symbol: "QNC", Quantity: py.Ptr(5), SlKind: "stop", SlPrice: py.Ptr(1.6)})
	b := s.GetBracket("bracket-1")
	if b.Status != "waiting" || b.Tif != "DAY" || b.SlTrailUnit != "pct" {
		t.Fatalf("%+v", b)
	}
	s.UpdateBracket("bracket-1", BracketPatch{"status": "armed", "slNative": true, "attempts": 2, "tpPrice": nil, "seenHeld": true})
	b = s.GetBracket("bracket-1")
	if b.Status != "armed" || !b.SlNative || b.Attempts != 2 || b.TpPrice != nil || !b.SeenHeld {
		t.Fatalf("%+v", b)
	}
	if len(s.ListBrackets([]string{"armed"})) != 1 || len(s.ListBrackets([]string{"waiting"})) != 0 || s.BracketForOrder("order-1") == nil {
		t.Error("list")
	}
}

func TestFilingsReplaceKeepsWhatWasRead(t *testing.T) {
	s := temp(t)
	s.ReplaceFilings("QNC", "SEDAR+", []FilingItem{{ID: "sedar:drm:1", Source: "SEDAR+", Type: "News release", Date: "2026-09-01T10:00"}}, "")
	s.SetFilingEnrichment("QNC", "sedar:drm:1", strPtr("Closing"), strPtr("A sentence."), intPtr(11), nil)
	s.ReplaceFilings("QNC", "SEDAR+", []FilingItem{{ID: "sedar:drm:1", Source: "SEDAR+", Type: "News release", Date: "2026-09-01T10:00", Size: "12 KB"}, {ID: "sedar:drm:2", Source: "SEDAR+", Type: "MD&A", Date: "2026-09-02T10:00"}}, "")
	rows := s.Filings("QNC")
	if len(rows) != 2 || rows[1].Subject != "Closing" || rows[1].Summary != "A sentence." || rows[1].EnrichVersion != 11 || rows[1].Size != "12 KB" {
		t.Fatalf("%+v", rows)
	}
	s.ReplaceFilings("QNC", "SEC", []FilingItem{{ID: "sec:1", Source: "SEC", Type: "8-K", Date: "2026-09-03"}}, "")
	if len(s.Filings("QNC")) != 3 {
		t.Error("another source's rows are untouched")
	}
	s.MarkFilingsFetched("QNC", "000123456", "")
	if s.SedarProfile("QNC") != "000123456" || s.FilingsFetchedAt("QNC") == "" {
		t.Error("stamps")
	}
}

func strPtr(s string) *string { return &s }
func intPtr(n int) *int       { return &n }

func TestWatchlistNewsShorts(t *testing.T) {
	s := temp(t)
	w := s.AddWatch("qnc", "tsx-v", "", "cad", "", "2026-01-01T00:00:00Z")
	if w.Symbol != "QNC" || w.Exchange != "TSX-V" || w.Currency != "CAD" {
		t.Fatalf("%+v", w)
	}
	w = s.AddWatch("QNC", "TSX-V", "Quantum eMotion", "", "", "")
	if w.Name != "Quantum eMotion" || w.AddedAt != "2026-01-01T00:00:00Z" {
		t.Fatalf("fills in what was blank, keeps its place: %+v", w)
	}
	s.ReplaceNews("QNC", "TSX-V", "tmx", []WireItem{{ID: "tmx:1", Headline: "H", Source: "GlobeNewswire", PublishedAt: "2026-09-01T10:00:00Z", Kind: "release"}}, "")
	if !s.HasWireRelease("QNC") || len(s.NewsIDs("QNC", "TSX-V")) != 1 {
		t.Error("news")
	}
	s.SaveShorts("QNC", "TSX-V", Short{Market: "ca", Shares: py.Ptr(1000), Series: []ShortPoint{{Date: "2026-08-31", Shares: py.Ptr(900)}}}, "", 5, true)
	s.SaveShorts("QNC", "TSX-V", Short{Market: "ca", Shares: py.Ptr(1100)}, "", 5, false)
	sh := s.ShortsFor("QNC", "TSX-V")
	if *sh.Shares != 1100 || len(sh.Series) != 1 {
		t.Fatalf("a read without a run keeps the stored run: %+v", sh)
	}
	if !s.RemoveWatch("QNC", "TSX-V") || s.RemoveWatch("QNC", "TSX-V") {
		t.Error("remove")
	}
}
