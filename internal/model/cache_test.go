package model

import (
	"encoding/json"
	"reflect"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/fixtures"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

func fixClock(t *testing.T, day string) {
	t.Helper()
	restore := Now
	t.Cleanup(func() { Now = restore })
	d, err := time.ParseInLocation("2006-01-02", day, TimeTZ)
	if err != nil {
		t.Fatal(err)
	}
	Now = func() time.Time { return d.Add(12 * time.Hour) }
}

func TestModelCacheRollsOverAtMidnight(t *testing.T) {
	m := New(tempStore(t))
	fixClock(t, "2026-12-31")
	m.Invalidate(false)
	if got := m.BaseModel(false).Today; got != "2026-12-31" {
		t.Errorf("today: %q", got)
	}
	if m.BaseModel(false) != m.BaseModel(false) {
		t.Error("same day: the cached base is reused")
	}
	fixClock(t, "2027-01-01")
	if got := m.BaseModel(false).Today; got != "2027-01-01" {
		t.Errorf("a new day rebuilds even though no data changed: %q", got)
	}
}

func TestModelViewFromStoreAndCache(t *testing.T) {
	st := tempStore(t)
	st.MergeLocalRows([]store.Activity{
		buy("b1", "AAA", 10, 1, "2026-01-01", fixtures.WithSource("csv")),
		sell("s1", "AAA", 10, 2, "2026-01-05", fixtures.WithSource("csv")),
	})
	m := New(st)
	v := viewJSON(t, m, nil, "")
	if mapAt(v, "kpi")["count"] != 1.0 {
		t.Errorf("count: %v", mapAt(v, "kpi")["count"])
	}
	almost(t, mapAt(v, "kpi")["realized"].(float64), 10, "realized")
	base1 := m.BaseModel(false)
	if base1 != m.BaseModel(false) {
		t.Error("the base is rebuilt without a change")
	}
	tid := listAt(v, "trades")[0].(map[string]any)["id"].(string)
	st.SaveJournalEntry(tid, map[string]any{"grade": "B"})
	v2 := viewJSON(t, m, nil, "")
	if got := listAt(v2, "trades")[0].(map[string]any)["grade"]; got != "B" {
		t.Errorf("grade: %v", got)
	}
	if got := listAt(mapAt(v2, "grades"), "buckets")[1].(map[string]any)["n"]; got != 1.0 {
		t.Errorf("B bucket: %v", got)
	}
}

func detailStore(t *testing.T) (*store.Store, *Model) {
	t.Helper()
	st := tempStore(t)
	m := New(st)
	m.Invalidate(false)
	st.UpsertFXRates(map[string]float64{"2099-01-01": 1.0})
	st.UpsertBenchmarkPrices(map[string]float64{"2099-01-01": 1.0}, "SP500")
	st.MergeLocalRows([]store.Activity{
		buy("b1", "AAA", 10, 1, "2026-01-01", fixtures.WithSource("csv")),
		sell("s1", "AAA", 10, 2, "2026-01-05", fixtures.WithSource("csv")),
		buy("b2", "BBB", 5, 3, "2026-01-02", fixtures.WithSource("csv")),
	})
	return st, m
}

func hasKey(m map[string]any, key string) bool {
	_, ok := m[key]
	return ok
}

func TestTheViewCarriesLegsAndFillsForTheOpenTradeOnly(t *testing.T) {
	_, m := detailStore(t)
	base := m.BaseModel(false)
	trade := base.Trades[0]
	holding := base.Positions[0]
	if len(trade.Fills) != 2 || len(holding.Fills) != 1 {
		t.Errorf("the base model keeps every fill: %d, %d", len(trade.Fills), len(holding.Fills))
	}
	v := viewJSON(t, m, nil, "")
	tr := listAt(v, "trades")[0].(map[string]any)
	pos := listAt(v, "positions")[0].(map[string]any)
	if hasKey(tr, "legs") || hasKey(tr, "fills") {
		t.Error("the trade row carries legs or fills")
	}
	if hasKey(pos, "legs") || hasKey(pos, "fills") {
		t.Error("the holding row carries legs or fills")
	}
	if tr["legCount"] != 1.0 {
		t.Errorf("the counts stay on the row: %v", tr["legCount"])
	}
	v = viewJSON(t, m, nil, trade.ID)
	if got := len(listAt(listAt(v, "trades")[0].(map[string]any), "fills")); got != 2 {
		t.Errorf("open trade fills: %d", got)
	}
	if hasKey(listAt(v, "positions")[0].(map[string]any), "fills") {
		t.Error("the holding carries fills while the trade is open")
	}
	v = viewJSON(t, m, nil, holding.ID)
	if got := len(listAt(listAt(v, "positions")[0].(map[string]any), "fills")); got != 1 {
		t.Errorf("open holding fills: %d", got)
	}
	if hasKey(listAt(v, "trades")[0].(map[string]any), "fills") {
		t.Error("the trade carries fills while the holding is open")
	}
	if len(base.Trades[0].Fills) != 2 {
		t.Error("slimming the view never touches the base model")
	}
}

func TestTradeDetailFindsATradeOrAHoldingByID(t *testing.T) {
	_, m := detailStore(t)
	base := m.BaseModel(false)
	d := TradeDetail(base, base.Trades[0].ID)
	if d == nil {
		t.Fatal("no detail for the trade")
	}
	ids := []string{}
	for _, f := range d.Fills {
		ids = append(ids, f.ID)
	}
	if d.ID != base.Trades[0].ID || len(d.Legs) != 1 || !reflect.DeepEqual(ids, []string{"s1", "b1"}) {
		t.Errorf("trade detail: %s", jsonOf(d))
	}
	d = TradeDetail(base, base.Positions[0].ID)
	if d == nil {
		t.Fatal("no detail for the holding")
	}
	ids = []string{}
	for _, f := range d.Fills {
		ids = append(ids, f.ID)
	}
	if len(d.Legs) != 0 || !reflect.DeepEqual(ids, []string{"b2"}) {
		t.Errorf("holding detail: %s", jsonOf(d))
	}
	if TradeDetail(base, "nope") != nil {
		t.Error("detail for an unknown id")
	}
	if TradeDetail(base, "") != nil {
		t.Error("detail for no id")
	}
}

func TestAQuoteTickReusesTheMatchedBook(t *testing.T) {
	st, m := detailStore(t)
	base := m.BaseModel(false)
	book := m.book
	st.UpsertQuote("AAA", store.Quote{Price: ptr(9.0)}, "tmx")
	ticked := m.BaseModel(false)
	if ticked == base {
		t.Error("a quote changes the data version, so the base is rebuilt")
	}
	if m.book != book {
		t.Error("but the activity rows are not matched again")
	}
	tickedIDs, baseIDs := []string{}, []string{}
	for _, tr := range ticked.Trades {
		tickedIDs = append(tickedIDs, tr.ID)
	}
	for _, tr := range base.Trades {
		baseIDs = append(baseIDs, tr.ID)
	}
	if !reflect.DeepEqual(tickedIDs, baseIDs) {
		t.Errorf("trade ids: %v vs %v", tickedIDs, baseIDs)
	}
	if !reflect.DeepEqual(ticked.Positions[0].Fills, base.Positions[0].Fills) {
		t.Errorf("fills: %s vs %s", jsonOf(ticked.Positions[0].Fills), jsonOf(base.Positions[0].Fills))
	}
	st.MergeLocalRows([]store.Activity{buy("b3", "CCC", 1, 4, "2026-01-03", fixtures.WithSource("csv"))})
	grown := m.BaseModel(false)
	if m.book == book {
		t.Error("a new activity row is matched")
	}
	symbols := []string{}
	for _, p := range grown.Positions {
		symbols = append(symbols, p.Symbol)
	}
	if got := sortedStrings(symbols); !reflect.DeepEqual(got, []string{"BBB", "CCC"}) {
		t.Errorf("positions: %v", got)
	}
	if grown.ActivityCount != 4 {
		t.Errorf("activity count: %d", grown.ActivityCount)
	}
}

func tickStore(t *testing.T) (*store.Store, *Model) {
	t.Helper()
	st := tempStore(t)
	m := New(st)
	m.Invalidate(true)
	st.SaveTiles([]store.Tile{})
	for _, a := range []store.Activity{
		buy("b1", "AAA", 100, 10, "2026-01-05", fixtures.WithAccount("Trading", "")),
		sell("s1", "AAA", 100, 12, "2026-02-05", fixtures.WithAccount("Trading", "")),
		buy("b2", "BBB", 20, 50, "2026-03-05", fixtures.WithAccount("Trading", "")),
	} {
		if _, err := st.InsertLocal(a); err != nil {
			t.Fatal(err)
		}
	}
	st.ReplaceAccounts([]store.Account{{ID: "acct-1", Nickname: "Trading", UnifiedAccountType: "TFSA", Currency: "CAD"}})
	st.UpsertQuote("BBB", store.Quote{Price: ptr(60.0), Currency: "CAD"}, "tmx")
	m.Invalidate(true)
	return st, m
}

func fullTrades(trades []*Trade) []tradeFull {
	out := []tradeFull{}
	for _, tr := range trades {
		out = append(out, tradeFull{tr.TradeCore, nonNil(tr.Legs), nonNilFills(tr.Fills)})
	}
	return out
}

func fullPositions(positions []*Position) []positionFull {
	out := []positionFull{}
	for _, p := range positions {
		out = append(out, positionFull{p.PositionCore, nonNilFills(p.Fills)})
	}
	return out
}

func slimView(t *testing.T, base *Base) map[string]any {
	t.Helper()
	var out map[string]any
	if err := json.Unmarshal(marshalView(BuildView(base, nil), ""), &out); err != nil {
		t.Fatal(err)
	}
	delete(out, "generated")
	return out
}

func TestAPriceTickMarksTheSameModelWithoutRematching(t *testing.T) {
	st, m := tickStore(t)
	first := m.BaseModel(false)
	if len(first.Positions) == 0 {
		t.Fatal("the open BBB lot is a position")
	}
	if len(first.Trades) == 0 {
		t.Fatal("and the closed AAA round trip is a trade")
	}
	book := m.book
	st.UpsertQuote("BBB", store.Quote{Price: ptr(70.0), Currency: "CAD"}, "tmx")
	marked := m.BaseModel(false)
	if m.book != book {
		t.Error("a price tick does not match the book again")
	}
	m.Invalidate(true)
	full := m.BaseModel(true)
	if !sameJSON(fullPositions(marked.Positions), fullPositions(full.Positions)) {
		t.Errorf("marked positions equal a full rebuild: %s vs %s", jsonOf(fullPositions(marked.Positions)), jsonOf(fullPositions(full.Positions)))
	}
	if !sameJSON(fullTrades(marked.Trades), fullTrades(full.Trades)) {
		t.Errorf("and so do the closed trades: %s vs %s", jsonOf(fullTrades(marked.Trades)), jsonOf(fullTrades(full.Trades)))
	}
	if !sameJSON(marked.Cashflow, full.Cashflow) {
		t.Errorf("cashflow: %s vs %s", jsonOf(marked.Cashflow), jsonOf(full.Cashflow))
	}
	if !sameJSON(marked.Equity, full.Equity) {
		t.Errorf("equity: %s vs %s", jsonOf(marked.Equity), jsonOf(full.Equity))
	}
	if !sameJSON(slimView(t, marked), slimView(t, full)) {
		t.Error("the page is served exactly what a full rebuild would have produced")
	}
}

func TestANewRowDoesMatchTheBookAgain(t *testing.T) {
	st, m := tickStore(t)
	m.BaseModel(false)
	book := m.book
	if _, err := st.InsertLocal(buy("b3", "CCC", 5, 20, "2026-04-05", fixtures.WithAccount("Trading", ""))); err != nil {
		t.Fatal(err)
	}
	m.BaseModel(false)
	if m.book == nil || m.book == book {
		t.Error("an activity row is a new book")
	}
}

func TestInvalidateKeepsTheMatchAndBookTrueDropsIt(t *testing.T) {
	_, m := tickStore(t)
	m.BaseModel(false)
	m.Invalidate(false)
	if m.book == nil {
		t.Error("a quote or a headline does not throw the match away")
	}
	m.Invalidate(true)
	if m.book != nil {
		t.Error("new rows do")
	}
}

func TestAGradeWrittenSurvivesTheNextPriceTick(t *testing.T) {
	st, m := tickStore(t)
	base := m.BaseModel(false)
	position := base.Positions[0]
	st.SaveJournalEntry(position.ID, map[string]any{"grade": "A", "thesis": "held", "tags": []any{"core"}})
	m.ApplyJournal(st.Journal())
	st.UpsertQuote("BBB", store.Quote{Price: ptr(80.0), Currency: "CAD"}, "tmx")
	marked := m.BaseModel(false)
	var again *Position
	for _, p := range marked.Positions {
		if p.ID == position.ID {
			again = p
		}
	}
	if again == nil {
		t.Fatal("the position is gone")
	}
	if again.Grade != "A" {
		t.Errorf("the grade is marked in, not dropped with the old journal: %q", again.Grade)
	}
	if !reflect.DeepEqual(again.Tags, []string{"core"}) {
		t.Errorf("tags: %v", again.Tags)
	}
}
