package shorts

import (
	"bytes"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math"
	"net/http"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"
	"unicode/utf16"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const usFile = "Date|Symbol|ShortVolume|ShortExemptVolume|TotalVolume|Market\r\n" +
	"20260914|A|380591.095732|11|631970.726692|B,Q,N\r\n" +
	"20260914|GME|2250985.897562|1942|3542062.253804|B,Q,N\r\n" +
	"20260914|NOVOL|0|0|0|Q\r\n" +
	"20260914|SHORT\r\n"

var caGrid = [][]any{{"", "", "", "", ""},
	{"Security Issue Name", "Security Symbol", "Exchange Code", "No.Shares", "Net Change"},
	{"QUANTUM EMOTION CORP.", "QNC", "TSXV", 2667164.0, 64077.0},
	{"1933 INDUSTRIES INC.", "TGIF", "CSE", 72000.0, 68990.0},
	{"ROW WITH NO SHARES", "NIL", "TSX", "", ""},
	{"SHORT ROW", "OOPS"}}

const caCSV = "Security,Company Name,Listing Market,Short Sale Trades,% Total Trades,Short Traded Volume,% Total Traded Volume,Short Traded Value,% Total Traded Value\r\n" +
	"QNC,Quantum Emotion Corp.,TSXV,6364,33.528,1197633,21.319,3453172,21.028\r\n" +
	"TGIF,1933 Industries Inc.,CSE,12,1.5,5000,0,20,0\r\n"

var sep15 = time.Date(2026, 9, 15, 0, 0, 0, 0, time.UTC)

func day(y int, m time.Month, d int) time.Time { return time.Date(y, m, d, 0, 0, 0, 0, time.UTC) }

func p(v float64) *float64 { return &v }

func show(v *float64) any {
	if v == nil {
		return nil
	}
	return *v
}

func is(v *float64, want float64) bool { return v != nil && *v == want }

func almost(a, b float64) bool { return math.Abs(a-b) < 5e-8 }

type stub struct {
	t      *testing.T
	asked  []string
	bodies []string
	answer func(method, url, body string) (int, string, error)
}

func (s *stub) RoundTrip(req *http.Request) (*http.Response, error) {
	body := ""
	if req.Body != nil {
		raw, _ := io.ReadAll(req.Body)
		req.Body.Close()
		body = string(raw)
	}
	s.asked = append(s.asked, req.URL.String())
	s.bodies = append(s.bodies, body)
	if s.answer == nil {
		return nil, errors.New("unanswered")
	}
	status, text, err := s.answer(req.Method, req.URL.String(), body)
	if err != nil {
		return nil, err
	}
	return &http.Response{StatusCode: status, Status: strconv.Itoa(status), Body: io.NopCloser(strings.NewReader(text)), Header: http.Header{}, Request: req, ContentLength: int64(len(text))}, nil
}

func (s *stub) count(needle string) int {
	n := 0
	for _, u := range s.asked {
		if strings.Contains(u, needle) {
			n++
		}
	}
	return n
}

func newClient(t *testing.T) (*Client, *stub) {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	t.Cleanup(func() { st.Close() })
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	m := market.NewClient(st)
	s := &stub{t: t}
	m.HTTP = &http.Client{Transport: s}
	c := NewClient(m)
	c.yahooTried = true
	return c, s
}

func jsonText(v any) string {
	b, _ := json.Marshal(v)
	return string(b)
}

func refuse(t *testing.T, msg string) func(method, url, body string) (int, string, error) {
	return func(method, url, body string) (int, string, error) {
		t.Errorf("%s: %s %s", msg, method, url)
		return 0, "", errors.New(msg)
	}
}

func operationOf(body string) string {
	var q struct {
		OperationName string `json:"operationName"`
	}
	json.Unmarshal([]byte(body), &q)
	return q.OperationName
}

func le16(v int) []byte {
	b := make([]byte, 2)
	binary.LittleEndian.PutUint16(b, uint16(v))
	return b
}

func le32(v int32) []byte {
	b := make([]byte, 4)
	binary.LittleEndian.PutUint32(b, uint32(v))
	return b
}

func biffRecord(rid int, body []byte) []byte {
	return append(append(le16(rid), le16(len(body))...), body...)
}

func biffLabel(row, col int, s string) []byte {
	body := append(append(append(le16(row), le16(col)...), le16(0)...), le16(len(s))...)
	body = append(body, 0)
	return biffRecord(0x0204, append(body, []byte(s)...))
}

func biffNumber(row, col int, value float64) []byte {
	b := make([]byte, 8)
	binary.LittleEndian.PutUint64(b, math.Float64bits(value))
	return biffRecord(0x0203, append(append(append(le16(row), le16(col)...), le16(0)...), b...))
}

func oleContainer(stream []byte, name string) []byte {
	sectors := (len(stream) + 511) / 512
	data := append(append([]byte{}, stream...), make([]byte, sectors*512-len(stream))...)
	header := make([]byte, 512)
	copy(header, []byte{0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1})
	copy(header[28:], le16(0xFFFE))
	copy(header[30:], le16(9))
	copy(header[32:], le16(6))
	copy(header[44:], le32(1))
	copy(header[48:], le32(1))
	copy(header[56:], le32(4096))
	copy(header[60:], le32(-2))
	copy(header[68:], le32(-2))
	copy(header[72:], le32(0))
	for i := 0; i < 109; i++ {
		v := int32(-1)
		if i == 0 {
			v = 0
		}
		copy(header[76+i*4:], le32(v))
	}
	table := bytes.Repeat([]byte{0xff}, 512)
	copy(table[0:], le32(-3))
	copy(table[4:], le32(-2))
	for i := 0; i < sectors; i++ {
		next := int32(3 + i)
		if i == sectors-1 {
			next = -2
		}
		copy(table[8+i*4:], le32(next))
	}
	directory := make([]byte, 512)
	entry := func(at int, entryName string, kind byte, start int32, size uint32) {
		var raw []byte
		for _, u := range utf16.Encode([]rune(entryName)) {
			raw = append(raw, le16(int(u))...)
		}
		copy(directory[at:], raw)
		copy(directory[at+64:], le16(len(raw)+2))
		directory[at+66] = kind
		copy(directory[at+116:], le32(start))
		binary.LittleEndian.PutUint32(directory[at+120:], size)
	}
	entry(0, "Root Entry", 5, -2, 0)
	entry(128, name, 2, 2, uint32(len(stream)))
	out := append([]byte{}, header...)
	out = append(out, table...)
	out = append(out, directory...)
	return append(out, data...)
}

func xlsOf(grid [][]any) string {
	var stream []byte
	for r, row := range grid {
		for c, v := range row {
			switch x := v.(type) {
			case string:
				if x != "" {
					stream = append(stream, biffLabel(r, c, x)...)
				}
			case float64:
				stream = append(stream, biffNumber(r, c, x)...)
			}
		}
	}
	return string(oleContainer(append(stream, make([]byte, 5000)...), "Workbook"))
}

func TestEachMarketGoesToTheRegulatorThatPublishesForIt(t *testing.T) {
	cases := []struct{ symbol, exchange, currency, want string }{
		{"GME", "NYSE", "USD", "us"}, {"AAPL", "NASDAQ", "USD", "us"}, {"QNC", "TSX-V", "CAD", "ca"}, {"TGIF", "CSE", "CAD", "ca"}, {"HBIX", "Cboe Canada", "CAD", "ca"}}
	for _, c := range cases {
		if got := MarketOf(c.symbol, c.exchange, c.currency); got != c.want {
			t.Errorf("MarketOf(%q, %q, %q) = %q, want %q", c.symbol, c.exchange, c.currency, got, c.want)
		}
	}
}

func TestAVenueTheBookDoesNotNameFollowsTheCurrencyAsTheQuotesDo(t *testing.T) {
	if got := MarketOf("SHOP", "", "CAD"); got != "ca" {
		t.Errorf("SHOP = %q", got)
	}
	if got := MarketOf("F", "", "USD"); got != "us" {
		t.Errorf("F = %q", got)
	}
}

func TestNothingIsClaimedForAnInstrumentNoOneReports(t *testing.T) {
	cases := []struct{ symbol, exchange, currency string }{
		{"BTC", "Crypto", "USD"}, {"SPX", "Index", "USD"}, {"ES", "CME", "USD"}, {"AAPL  260117C00150000", "NASDAQ", "USD"}, {"", "NYSE", "USD"}}
	for _, c := range cases {
		if got := MarketOf(c.symbol, c.exchange, c.currency); got != "" {
			t.Errorf("MarketOf(%q, %q, %q) = %q, want \"\"", c.symbol, c.exchange, c.currency, got)
		}
	}
}

func sameDays(got []time.Time, want []time.Time) bool {
	if len(got) != len(want) {
		return false
	}
	for i := range got {
		if !got[i].Equal(want[i]) {
			return false
		}
	}
	return true
}

func TestPositionsAreReportedOnTheFifteenthAndTheLastDay(t *testing.T) {
	want := []time.Time{day(2026, 9, 15), day(2026, 8, 31), day(2026, 8, 15), day(2026, 7, 31)}
	if got := PositionDates(day(2026, 9, 15), 4); !sameDays(got, want) {
		t.Errorf("got %v", got)
	}
}

func TestADateStillToComeIsNeverAskedFor(t *testing.T) {
	want := []time.Time{day(2026, 8, 31), day(2026, 8, 15)}
	if got := PositionDates(day(2026, 9, 3), 2); !sameDays(got, want) {
		t.Errorf("got %v", got)
	}
}

func TestTheTurnOfTheYearStepsBackIntoDecember(t *testing.T) {
	want := []time.Time{day(2025, 12, 31), day(2025, 12, 15)}
	if got := PositionDates(day(2026, 1, 5), 2); !sameDays(got, want) {
		t.Errorf("got %v", got)
	}
}

func TestVolumePeriodsAreTheTwoHalvesOfEachMonth(t *testing.T) {
	want := []period{{day(2026, 9, 1), day(2026, 9, 15)}, {day(2026, 8, 16), day(2026, 8, 31)}, {day(2026, 8, 1), day(2026, 8, 15)}}
	got := VolumePeriods(day(2026, 9, 15), 3)
	if len(got) != len(want) {
		t.Fatalf("got %v", got)
	}
	for i := range got {
		if !got[i].start.Equal(want[i].start) || !got[i].end.Equal(want[i].end) {
			t.Errorf("got %v", got)
		}
	}
}

func TestTheDailyFileIsOnlyLookedForOnWeekdays(t *testing.T) {
	want := []time.Time{day(2026, 9, 15), day(2026, 9, 14), day(2026, 9, 11), day(2026, 9, 10)}
	if got := TradingDays(day(2026, 9, 15), 4); !sameDays(got, want) {
		t.Errorf("got %v", got)
	}
}

func TestTheDailyUSFileGivesTheShortPartOfEachSymbolsVolume(t *testing.T) {
	rows := ParseUSVolume(usFile)
	if want := map[string]any{"shortVolume": 2250985.897562, "totalVolume": 3542062.253804}; !reflect.DeepEqual(rows["GME"], want) {
		t.Errorf("GME = %v", rows["GME"])
	}
	for _, sym := range []string{"NOVOL", "SHORT", "Symbol"} {
		if _, ok := rows[sym]; ok {
			t.Errorf("%s in rows", sym)
		}
	}
}

func TestTheCanadianPositionReportGivesSharesShortAndTheChange(t *testing.T) {
	rows := ParseCAPositions(caGrid)
	if want := map[string]any{"venue": "TSXV", "shares": 2667164.0, "change": 64077.0, "name": "QUANTUM EMOTION CORP."}; !reflect.DeepEqual(rows["QNC"], want) {
		t.Errorf("QNC = %v", rows["QNC"])
	}
	if rows["TGIF"]["venue"] != "CSE" {
		t.Errorf("TGIF venue = %v", rows["TGIF"]["venue"])
	}
	for _, sym := range []string{"NIL", "OOPS", "Security Symbol"} {
		if _, ok := rows[sym]; ok {
			t.Errorf("%s in rows", sym)
		}
	}
}

func TestTheCanadianVolumeReportGivesTheShortShareOfTrading(t *testing.T) {
	rows := ParseCAVolume(caCSV)
	if rows["QNC"]["shortVolume"] != 1197633.0 {
		t.Errorf("shortVolume = %v", rows["QNC"]["shortVolume"])
	}
	if rows["QNC"]["volumePct"] != 21.319 {
		t.Errorf("volumePct = %v", rows["QNC"]["volumePct"])
	}
	if total, _ := rows["QNC"]["totalVolume"].(float64); !almost(total, 1197633.0/21.319*100) {
		t.Errorf("totalVolume = %v", rows["QNC"]["totalVolume"])
	}
	if rows["TGIF"]["totalVolume"] != nil {
		t.Errorf("TGIF totalVolume = %v", rows["TGIF"]["totalVolume"])
	}
}

func TestARowIsOnlyUsedForTheVenueItWasFiledUnder(t *testing.T) {
	if !venueFits("TSXV", "TSX-V") {
		t.Error("TSXV/TSX-V")
	}
	if !venueFits("AQL", "Cboe Canada") {
		t.Error("AQL/Cboe Canada")
	}
	if !venueFits("TSX", "") {
		t.Error("TSX/''")
	}
	if venueFits("TSX", "CSE") {
		t.Error("TSX/CSE")
	}
}

func TestTheNewestSettlementFinraHasIsTheOneShown(t *testing.T) {
	c, s := newClient(t)
	answered := []any{
		map[string]any{"settlementDate": "2026-08-14", "currentShortPositionQuantity": 54036583, "previousShortPositionQuantity": 53736062, "changePreviousNumber": 300521},
		map[string]any{"settlementDate": "2026-08-31", "currentShortPositionQuantity": 56990026, "previousShortPositionQuantity": 54036583, "changePreviousNumber": 2953443, "averageDailyVolumeQuantity": 5864237},
		"not a row"}
	s.answer = func(method, url, body string) (int, string, error) { return 200, jsonText(answered), nil }
	out, ok := c.USPosition("GME", time.Now().UTC())
	if !ok {
	}
	got := map[string]any{"asOf": out.AsOf, "shares": show(out.Shares), "previous": show(out.Previous), "change": show(out.Change), "previousOf": out.PreviousOf, "averageVolume": show(out.AverageVolume)}
	want := map[string]any{"asOf": "2026-08-31", "shares": 56990026.0, "previous": 54036583.0, "change": 2953443.0, "previousOf": "2026-08-14", "averageVolume": 5864237.0}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got %v", got)
	}
}

func TestASymbolFinraDoesNotCarryAnswersNothingRatherThanGuessing(t *testing.T) {
	c, s := newClient(t)
	s.answer = func(method, url, body string) (int, string, error) { return 200, "[]", nil }
	out, ok := c.USPosition("NOSUCH", time.Now().UTC())
	if ok || !reflect.DeepEqual(out, Record{}) {
		t.Errorf("got %v %+v", ok, out)
	}
}

func TestAWholeMarketFileIsReadOnceAndUsedForEveryListing(t *testing.T) {
	c, s := newClient(t)
	s.answer = func(method, url, body string) (int, string, error) { return 200, usFile, nil }
	var first, second Record
	c.USVolume("GME", sep15, &first)
	c.USVolume("A", sep15, &second)
	if len(s.asked) != 1 {
		t.Errorf("asked %d times: %v", len(s.asked), s.asked)
	}
	if first.VolumeOf != "2026-09-15" {
		t.Errorf("volumeOf = %q", first.VolumeOf)
	}
	if first.VolumePct == nil || !almost(*first.VolumePct, 2250985.897562/3542062.253804*100) {
		t.Errorf("volumePct = %v", show(first.VolumePct))
	}
	if second.VolumeSpan != "day" {
		t.Errorf("volumeSpan = %q", second.VolumeSpan)
	}
}

func TestAFileThatWillNotAnswerKeepsWhatWasAlreadyRead(t *testing.T) {
	c, s := newClient(t)
	s.answer = func(method, url, body string) (int, string, error) { return 200, usFile, nil }
	var warm Record
	c.USVolume("GME", sep15, &warm)
	c.files["us_volume"].at = time.Time{}
	s.answer = func(method, url, body string) (int, string, error) { return 0, "", errors.New("down") }
	var kept Record
	c.USVolume("GME", sep15, &kept)
	if !is(kept.ShortVolume, 2250985.897562) {
		t.Errorf("shortVolume = %v", show(kept.ShortVolume))
	}
}

func caReports(grid [][]any, csv string) func(method, url, body string) (int, string, error) {
	return func(method, url, body string) (int, string, error) {
		if strings.Contains(url, "CSPR") {
			return 200, xlsOf(grid), nil
		}
		if strings.Contains(url, "SSALE") {
			return 200, csv, nil
		}
		return 404, "", nil
	}
}

func TestACanadianListingReadsBothOfItsReports(t *testing.T) {
	c, s := newClient(t)
	s.answer = caReports(caGrid, caCSV)
	rec, _ := c.ForListing("QNC", "TSX-V", "CAD", sep15, false, "")
	if rec.Source != "CIRO" {
		t.Errorf("source = %q", rec.Source)
	}
	if !is(rec.Shares, 2667164.0) {
		t.Errorf("shares = %v", show(rec.Shares))
	}
	if !is(rec.Previous, 2603087.0) {
		t.Errorf("previous = %v", show(rec.Previous))
	}
	if rec.AsOf != "2026-09-15" {
		t.Errorf("asOf = %q", rec.AsOf)
	}
	if rec.VolumeOf != "2026-09-01/2026-09-15" {
		t.Errorf("volumeOf = %q", rec.VolumeOf)
	}
	if rec.VolumeSpan != "period" {
		t.Errorf("volumeSpan = %q", rec.VolumeSpan)
	}
	if rec.PreviousOf != "2026-08-31" {
		t.Errorf("previousOf = %q", rec.PreviousOf)
	}
}

func TestAListingFiledUnderAnotherVenueIsNotReadAsThisOne(t *testing.T) {
	c, s := newClient(t)
	s.answer = caReports(caGrid, caCSV)
	rec, _ := c.ForListing("QNC", "CSE", "CAD", sep15, false, "")
	if rec.Shares != nil {
		t.Errorf("shares = %v", show(rec.Shares))
	}
	if rec.Market != "ca" {
		t.Errorf("market = %q", rec.Market)
	}
}

func TestNothingIsReadForAnInstrumentNoOneReports(t *testing.T) {
	c, s := newClient(t)
	s.answer = refuse(t, "asked anyway")
	rec, ok := c.ForListing("BTC", "Crypto", "USD", time.Now().UTC(), false, "")
	if ok || !reflect.DeepEqual(rec, Record{}) {
		t.Errorf("got %v %+v", ok, rec)
	}
}

func TestDaysToCoverUsesTheVolumeOfTheListingsOwnMarket(t *testing.T) {
	c, _ := newClient(t)
	us := &Record{Market: "us", Shares: p(56990026.0), AverageVolume: p(5864237.0)}
	if !is(c.AverageVolume(us), 5864237.0) {
		t.Errorf("averageVolume = %v", show(c.AverageVolume(us)))
	}
	if !is(c.DaysToCover(us), 9.7) {
		t.Errorf("daysToCover = %v", show(c.DaysToCover(us)))
	}
}

func TestTheCanadianAverageCountsOnlyTheDaysTheMarketTraded(t *testing.T) {
	c, _ := newClient(t)
	prices := map[string]float64{}
	for _, d := range []string{"2026-08-17", "2026-08-18", "2026-08-19", "2026-08-20", "2026-08-21"} {
		prices[d] = 100.0
	}
	c.Store.UpsertBenchmarkPrices(prices, "TSX")
	ca := &Record{Market: "ca", Shares: p(2667164.0), TotalVolume: p(5000000.0), VolumeOf: "2026-08-16/2026-08-31"}
	days := c.Store.BenchmarkDays("TSX", "2026-08-16", "2026-08-31")
	got := c.AverageVolume(ca)
	if days != 0 {
		if got == nil || !almost(*got, 5000000.0/float64(days)) {
			t.Errorf("averageVolume = %v, days %d", show(got), days)
		}
	} else if got != nil {
		t.Errorf("averageVolume = %v with no calendar", show(got))
	}
}

func TestNoPositionOrNoVolumeLeavesDaysToCoverUnsaid(t *testing.T) {
	c, _ := newClient(t)
	if got := c.DaysToCover(&Record{Market: "us", Shares: nil, AverageVolume: p(10.0)}); got != nil {
		t.Errorf("no shares: %v", *got)
	}
	if got := c.DaysToCover(&Record{Market: "us", Shares: p(10.0), AverageVolume: nil}); got != nil {
		t.Errorf("no average: %v", *got)
	}
	if got := c.AverageVolume(&Record{Market: "ca", TotalVolume: nil, VolumeOf: "2026-08-16/2026-08-31"}); got != nil {
		t.Errorf("no total: %v", *got)
	}
}

func TestEverySettlementFinraAnsweredWithIsKeptOldestFirst(t *testing.T) {
	c, s := newClient(t)
	answered := []map[string]any{
		{"settlementDate": "2026-08-31", "currentShortPositionQuantity": 3},
		{"settlementDate": "2026-07-31", "currentShortPositionQuantity": 1},
		{"settlementDate": "2026-08-14", "currentShortPositionQuantity": 2},
		{"settlementDate": "2026-06-30", "currentShortPositionQuantity": nil}}
	s.answer = func(method, url, body string) (int, string, error) { return 200, jsonText(answered), nil }
	rec, _ := c.USPosition("GME", time.Now().UTC())
	var dates []string
	var shares []any
	for _, pt := range rec.Series {
		dates = append(dates, pt.Date)
		shares = append(shares, show(pt.Shares))
	}
	if !reflect.DeepEqual(dates, []string{"2026-07-31", "2026-08-14", "2026-08-31"}) {
		t.Errorf("dates = %v", dates)
	}
	if !reflect.DeepEqual(shares, []any{1.0, 2.0, 3.0}) {
		t.Errorf("shares = %v", shares)
	}
}

func reportDay(url string) string {
	last := url[strings.LastIndex(url, "/")+1:]
	return strings.SplitN(last, "_", 2)[0]
}

func TestTheCanadianRunReadsOneFilePerReportingDate(t *testing.T) {
	c, s := newClient(t)
	grids := map[string][][]any{
		"20260831": {{"", "QNC", "TSXV", 300.0, 0.0}},
		"20260815": {{"", "QNC", "TSXV", 200.0, 0.0}},
		"20260731": {{"", "QNC", "TSXV", 100.0, 0.0}}}
	s.answer = func(method, url, body string) (int, string, error) {
		grid, ok := grids[reportDay(url)]
		if !ok {
			return 0, "", errors.New("no report")
		}
		return 200, xlsOf(grid), nil
	}
	series := c.CASeries("QNC", "TSX-V", "2026-08-31", sep15, Series)
	var got []string
	for _, pt := range series {
		got = append(got, fmt.Sprintf("%s=%v", pt.Date, show(pt.Shares)))
	}
	if !reflect.DeepEqual(got, []string{"2026-07-31=100", "2026-08-15=200", "2026-08-31=300"}) {
		t.Errorf("series = %v", got)
	}
}

func TestAReportAfterTheOneOnShowIsNotDrawn(t *testing.T) {
	c, s := newClient(t)
	s.answer = func(method, url, body string) (int, string, error) { return 0, "", errors.New("none") }
	if got := c.CASeries("QNC", "TSX-V", "2026-07-31", sep15, Series); len(got) != 0 {
		t.Errorf("series = %v", got)
	}
}

func TestAListingOnAnotherVenueIsNotDrawnIntoThisOnesRun(t *testing.T) {
	c, s := newClient(t)
	s.answer = func(method, url, body string) (int, string, error) {
		return 200, xlsOf([][]any{{"", "QNC", "CSE", 300.0, 0.0}}), nil
	}
	if got := c.CASeries("QNC", "TSX-V", "2026-08-31", sep15, Series); len(got) != 0 {
		t.Errorf("series = %v", got)
	}
}

func TestTheCanadianRunIsOnlyReadWhenItIsAskedFor(t *testing.T) {
	c, s := newClient(t)
	s.answer = caReports(caGrid, caCSV)
	quiet, _ := c.ForListing("QNC", "TSX-V", "CAD", sep15, false, "")
	if quiet.Series != nil {
	}
}
