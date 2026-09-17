package market

import (
	"encoding/json"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/browserhttp"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

const (
	CoinbaseExchangeProductURL = "https://api.exchange.coinbase.com/products/%s"
	CoinbaseCandlesURL         = "https://api.exchange.coinbase.com/products/%s/candles?granularity=%d&start=%s&end=%s"
	CoinbaseCandleLimit        = 300
	CoinbaseExchangeStart      = "2015-01-01"
	YahooChartURL              = "https://query1.finance.yahoo.com/v8/finance/chart/%s?period1=%d&period2=%d&interval=%s"
	YahooIntradayDays          = 729
	YahooMinIntervalSec        = 2.0
	YahooBackoffSec            = 600
	TMXIntradayDays            = 365
	SessionOpenMinutes         = 9*60 + 30
	HistoryStaleHours          = 20
	CoverageSlackDays          = 7
	IntradayRetryMinutes       = 10
	ArchiveBatch               = 12
	ArchiveTopupHours          = 20
)

var YahooHeaders = map[string]string{"User-Agent": "Mozilla/5.0", "Accept": "application/json"}
var SourceIntradayDays = map[string]int{"tmx": TMXIntradayDays, "yahoo": YahooIntradayDays}
var Timeframes = []string{"1h", "4h", "1d", "1w", "1M"}
var IntradaySeconds = map[string]int64{"1h": 3600, "4h": 14400}
var OnDemandOnlySources = []string{"yahoo"}
var ShortDailySources = []string{}

type Daily = store.DailyBar
type Bar = store.Bar

func ParseTMXHistory(data map[string]any) []Daily {
	d, _ := data["data"].(map[string]any)
	rows, _ := d["getTimeSeriesData"].([]any)
	out := []Daily{}
	for _, raw := range rows {
		r, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		day := py.S(r["dateTime"])
		if len(day) > 10 {
			day = day[:10]
		}
		if len(day) == 10 {
			out = append(out, Daily{Date: day, Open: numPtr(r["open"]), High: numPtr(r["high"]), Low: numPtr(r["low"]), Close: py.Num(r["close"], 0), Volume: numPtr(r["volume"])})
		}
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Date < out[j].Date })
	return out
}

func ParseCoinbaseCandles(text string) []Bar {
	var rows []any
	if text == "" {
		text = "[]"
	}
	if err := json.Unmarshal([]byte(text), &rows); err != nil {
		return []Bar{}
	}
	out := map[int64]Bar{}
	for _, raw := range rows {
		r, ok := raw.([]any)
		if !ok || len(r) < 6 {
			continue
		}
		tf, ok := py.NumOK(r[0])
		if !ok {
			continue
		}
		vals := make([]float64, 5)
		good := true
		for i := 1; i < 6; i++ {
			v, ok := py.NumOK(r[i])
			if !ok {
				good = false
				break
			}
			vals[i-1] = v
		}
		if !good {
			continue
		}
		lo, hi, op, cl, vol := vals[0], vals[1], vals[2], vals[3], vals[4]
		if cl > 0 {
			t := int64(tf)
			out[t] = Bar{Time: t, Open: py.Ptr(op), High: py.Ptr(hi), Low: py.Ptr(lo), Close: cl, Volume: py.Ptr(vol)}
		}
	}
	return sortedBars(out)
}

func sortedBars(m map[int64]Bar) []Bar {
	keys := make([]int64, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Slice(keys, func(i, j int) bool { return keys[i] < keys[j] })
	out := make([]Bar, 0, len(keys))
	for _, k := range keys {
		out = append(out, m[k])
	}
	return out
}

func (c *Client) CoinbaseMarket(pair string, now time.Time) string {
	pair = strings.ToUpper(strings.TrimSpace(pair))
	if !strings.Contains(pair, "-") {
		return ""
	}
	metaKey := "coinbase_product:" + pair
	v := c.Store.GetMeta(metaKey)
	if strings.HasPrefix(v, "@") {
		return v[1:]
	}
	today := py.DateStr(now)
	if strings.HasPrefix(v, "none@") && v[5:] > py.DateStr(now.AddDate(0, 0, -TMXResolveRetryDays)) {
		return ""
	}
	var d map[string]any
	if text, err := c.GetText(fmt.Sprintf(CoinbaseExchangeProductURL, pair), nil); err == nil {
		json.Unmarshal([]byte(text), &d)
	}
	if strings.ToUpper(py.S(d["id"])) == pair {
		c.Store.SetMeta(metaKey, "@"+pair)
		return pair
	}
	c.Store.SetMeta(metaKey, "none@"+today)
	return ""
}

func parallelMap[T any, R any](items []T, workers int, fn func(T) R) []R {
	out := make([]R, len(items))
	sem := make(chan struct{}, workers)
	var wg sync.WaitGroup
	for i := range items {
		wg.Add(1)
		sem <- struct{}{}
		go func(i int) {
			defer wg.Done()
			out[i] = fn(items[i])
			<-sem
		}(i)
	}
	wg.Wait()
	return out
}

func (c *Client) FetchCoinbaseCandles(product string, granularity int64, startTs, endTs int64) []Bar {
	span := CoinbaseCandleLimit * granularity
	type chunk struct{ a, b int64 }
	var chunks []chunk
	cur := startTs / granularity * granularity
	for cur < endTs {
		end := cur + span
		if end > endTs {
			end = endTs
		}
		chunks = append(chunks, chunk{cur, end})
		cur += span
	}
	iso := func(ts int64) string { return stamp(time.Unix(ts, 0)) }
	results := parallelMap(chunks, 4, func(ch chunk) []Bar {
		text, err := c.GetText(fmt.Sprintf(CoinbaseCandlesURL, product, granularity, iso(ch.a), iso(ch.b)), nil)
		if err != nil {
			return nil
		}
		return ParseCoinbaseCandles(text)
	})
	out := map[int64]Bar{}
	for _, bars := range results {
		for _, b := range bars {
			out[b.Time] = b
		}
	}
	return sortedBars(out)
}

func rateOnOrBefore(fx map[string]float64, day string, days int) *float64 {
	t, ok := py.ParseDate(day)
	if !ok {
		return nil
	}
	for i := 0; i < days; i++ {
		if r, ok := fx[py.DateStr(t.AddDate(0, 0, -i))]; ok && r > 0 {
			return &r
		}
	}
	return nil
}

func scale(p *float64, rate float64) *float64 {
	if p == nil {
		return nil
	}
	return py.Ptr(*p * rate)
}

func (c *Client) InPositionCurrencyDaily(bars []Daily, barCurrency, currency string) []Daily {
	quote := strings.ToUpper(barCurrency)
	ccy := strings.ToUpper(currency)
	if ccy == "" {
		ccy = "CAD"
	}
	if quote == ccy {
		return append([]Daily{}, bars...)
	}
	if !(quote == "USD" && ccy == "CAD") {
		return []Daily{}
	}
	fx := c.Store.FXRates()
	out := []Daily{}
	for _, b := range bars {
		rate := rateOnOrBefore(fx, b.Date, 7)
		if rate == nil {
			continue
		}
		nb := b
		nb.Open, nb.High, nb.Low, nb.Close = scale(b.Open, *rate), scale(b.High, *rate), scale(b.Low, *rate), b.Close**rate
		out = append(out, nb)
	}
	return out
}

func (c *Client) InPositionCurrencyBars(bars []Bar, barCurrency, currency string) []Bar {
	quote := strings.ToUpper(barCurrency)
	ccy := strings.ToUpper(currency)
	if ccy == "" {
		ccy = "CAD"
	}
	if quote == ccy {
		return append([]Bar{}, bars...)
	}
	if !(quote == "USD" && ccy == "CAD") {
		return []Bar{}
	}
	fx := c.Store.FXRates()
	out := []Bar{}
	for _, b := range bars {
		day := py.DateStr(time.Unix(b.Time, 0).UTC())
		rate := rateOnOrBefore(fx, day, 7)
		if rate == nil {
			continue
		}
		nb := b
		nb.Open, nb.High, nb.Low, nb.Close = scale(b.Open, *rate), scale(b.High, *rate), scale(b.Low, *rate), b.Close**rate
		out = append(out, nb)
	}
	return out
}

func ChartInstrument(rec Rec) Rec {
	if rec.Kind == "Options" {
		under := symbols.Underlying(rec.Symbol)
		if under != "" && under != "—" {
			ccy := rec.Currency
			if ccy == "" {
				ccy = "USD"
			}
			return Rec{Symbol: under, Exchange: rec.Exchange, Currency: ccy, Kind: "Shares"}
		}
	}
	return rec
}

type Candidate struct {
	Source string
	Key    string
}

func HistoryCandidates(rec Rec) []Candidate {
	kind := rec.Kind
	if kind == "" {
		kind = "Shares"
	}
	sym := TMXSymbol(rec.Symbol)
	ccy := strings.ToUpper(strings.TrimSpace(rec.Currency))
	if ccy == "" {
		ccy = "CAD"
	}
	if sym == "" {
		return nil
	}
	if kind == "Crypto" {
		out := []Candidate{{"coinbase", sym + "-" + ccy}, {"yahoo", sym + "-" + ccy}}
		if ccy != "USD" {
			out = append(out, Candidate{"coinbase", sym + "-USD"}, Candidate{"yahoo", sym + "-USD"})
		}
		return out
	}
	if kind != "Shares" {
		return nil
	}
	var out []Candidate
	if k := TMXQuoteSymbol(rec.Symbol, rec.Exchange, ccy); k != "" {
		out = append(out, Candidate{"tmx", k})
	}
	for _, f := range YahooFormsFor(rec) {
		out = append(out, Candidate{"yahoo", f})
	}
	return out
}

func HistorySource(rec Rec) *Candidate {
	if c := HistoryCandidates(rec); len(c) > 0 {
		return &c[0]
	}
	return nil
}

func barsMetaKey(rec Rec) string { return "bars_source:" + TMXSymbol(rec.Symbol) }

func (c *Client) OrderedCandidates(rec Rec) []Candidate {
	cands := HistoryCandidates(rec)
	if len(cands) == 0 {
		return cands
	}
	v := c.Store.GetMeta(barsMetaKey(rec))
	if src, key, ok := strings.Cut(v, "|"); ok {
		win := Candidate{src, key}
		for _, cand := range cands {
			if cand == win {
				out := []Candidate{win}
				for _, other := range cands {
					if other != win {
						out = append(out, other)
					}
				}
				return out
			}
		}
	}
	return cands
}

func (c *Client) rememberWinner(rec Rec, source, key string) {
	c.Store.SetMeta(barsMetaKey(rec), source+"|"+key)
}

func BarCurrency(source, key string, rec Rec) string {
	if rec.Kind == "Crypto" && strings.Contains(key, "-") {
		_, after, _ := strings.Cut(key, "-")
		return after
	}
	ccy := strings.ToUpper(rec.Currency)
	if ccy == "" {
		ccy = "CAD"
	}
	return ccy
}

type MinuteBar struct {
	Time   int64
	Day    string
	Minute int
	Offset int
	Open   *float64
	High   *float64
	Low    *float64
	Close  float64
	Volume *float64
}

func (m MinuteBar) bar() Bar {
	return Bar{Time: m.Time, Open: m.Open, High: m.High, Low: m.Low, Close: m.Close, Volume: m.Volume}
}

func ParseYahooChart(text string) []MinuteBar {
	var d map[string]any
	if text == "" {
		text = "{}"
	}
	if err := json.Unmarshal([]byte(text), &d); err != nil {
		return []MinuteBar{}
	}
	chart, _ := d["chart"].(map[string]any)
	results, _ := chart["result"].([]any)
	if len(results) == 0 {
		return []MinuteBar{}
	}
	r, _ := results[0].(map[string]any)
	ts, _ := r["timestamp"].([]any)
	indicators, _ := r["indicators"].(map[string]any)
	quotes, _ := indicators["quote"].([]any)
	q := map[string]any{}
	if len(quotes) > 0 {
		if qq, ok := quotes[0].(map[string]any); ok {
			q = qq
		}
	}
	meta, _ := r["meta"].(map[string]any)
	var tz *time.Location
	if name := py.S(meta["exchangeTimezoneName"]); name != "" {
		if loc, err := time.LoadLocation(name); err == nil {
			tz = loc
		}
	}
	fixed := int(py.Num(meta["gmtoffset"], 0))
	series := func(k string) []any {
		v, _ := q[k].([]any)
		return v
	}
	closes, opens, highs, lows, vols := series("close"), series("open"), series("high"), series("low"), series("volume")
	at := func(list []any, i int) *float64 {
		if i >= len(list) {
			return nil
		}
		return numPtr(list[i])
	}
	out := []MinuteBar{}
	for i, raw := range ts {
		tf, ok := py.NumOK(raw)
		if !ok {
			continue
		}
		t := int64(tf)
		cl := at(closes, i)
		if cl == nil || *cl <= 0 {
			continue
		}
		var local time.Time
		var off int
		if tz != nil {
			local = time.Unix(t, 0).In(tz)
			_, off = local.Zone()
		} else {
			off = fixed
			local = time.Unix(t+int64(off), 0).UTC()
		}
		out = append(out, MinuteBar{Time: t, Day: py.DateStr(local), Minute: local.Hour()*60 + local.Minute(), Offset: off, Open: at(opens, i), High: at(highs, i), Low: at(lows, i), Close: *cl, Volume: at(vols, i)})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Time < out[j].Time })
	return out
}

func (c *Client) yahooGet(rawURL string) (string, error) {
	c.yahooMu.Lock()
	defer c.yahooMu.Unlock()
	now := time.Now()
	if now.Before(c.yahooBackoffUntil) {
		c.NoteSource("yahoo", false, ErrBackingOff)
		return "", ErrBackingOff
	}
	if wait := c.yahooNextAt.Sub(now); wait > 0 {
		time.Sleep(wait)
	}
	c.yahooNextAt = time.Now().Add(time.Duration(YahooMinIntervalSec * float64(time.Second)))
	text, err := c.GetText(rawURL, YahooHeaders)
	if err != nil {
		if StatusOf(err) == 429 {
			c.yahooBackoffUntil = time.Now().Add(YahooBackoffSec * time.Second)
		}
		return "", err
	}
	return text, nil
}

func (c *Client) SetYahooBackoff(until time.Time) {
	c.yahooMu.Lock()
	c.yahooBackoffUntil = until
	c.yahooMu.Unlock()
}

func (c *Client) FetchYahoo(symbol string, startTs, endTs int64, interval string) ([]MinuteBar, error) {
	today := py.DateStr(c.now())
	missKey := "yahoo_miss:" + symbol
	if c.Store.GetMeta(missKey) == today {
		return []MinuteBar{}, nil
	}
	text, err := c.yahooGet(fmt.Sprintf(YahooChartURL, symbol, startTs, endTs, interval))
	if err != nil {
		if StatusOf(err) == 404 {
			c.Store.SetMeta(missKey, today)
			return []MinuteBar{}, nil
		}
		return nil, err
	}
	return ParseYahooChart(text), nil
}

func wholeDaily(bars []Daily) []Daily {
	out := []Daily{}
	for _, b := range bars {
		if b.Open != nil && b.High != nil && b.Low != nil {
			out = append(out, b)
		}
	}
	return out
}

func wholeMinutes(bars []MinuteBar) []MinuteBar {
	out := []MinuteBar{}
	for _, b := range bars {
		if b.Open != nil && b.High != nil && b.Low != nil {
			out = append(out, b)
		}
	}
	return out
}

func dayTs(day string) int64 {
	t, _ := py.ParseDate(day)
	return t.Unix()
}

func (c *Client) tmxLookupDaily(key string, fn func(form string) ([]Daily, error)) ([]Daily, error) {
	first := c.TMXRemembered(key)
	r, err := fn(first)
	if err != nil {
		return nil, err
	}
	if len(r) > 0 || key == "" || strings.HasPrefix(key, "^") {
		return r, nil
	}
	alt := c.TMXResolve(key)
	if alt != "" && alt != first {
		return fn(alt)
	}
	return r, nil
}

func (c *Client) FetchDailyFrom(source, key string, rec Rec, start, end string) ([]Daily, error) {
	startTs := dayTs(start)
	endTs := dayTs(end) + 86400
	switch source {
	case "tmx":
		bars, err := c.tmxLookupDaily(key, func(form string) ([]Daily, error) {
			data, err := c.PostJSON(TMXURL, map[string]any{"operationName": "getTimeSeriesData", "variables": map[string]any{"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end}, "query": TMXHistoryQuery}, TMXHeaders)
			if err != nil {
				return nil, err
			}
			return ParseTMXHistory(data), nil
		})
		if err != nil {
			return nil, err
		}
		return wholeDaily(bars), nil
	case "coinbase":
		if c.CoinbaseMarket(key, c.now()) == "" {
			return []Daily{}, nil
		}
		days := []Daily{}
		for _, b := range c.FetchCoinbaseCandles(key, 86400, startTs, endTs) {
			days = append(days, Daily{Date: py.DateStr(time.Unix(b.Time, 0).UTC()), Open: b.Open, High: b.High, Low: b.Low, Close: b.Close, Volume: b.Volume})
		}
		return c.InPositionCurrencyDaily(days, BarCurrency(source, key, rec), rec.Currency), nil
	case "yahoo":
		mins, err := c.FetchYahoo(key, startTs, endTs, "1d")
		if err != nil {
			return nil, err
		}
		days := []Daily{}
		for _, b := range mins {
			days = append(days, Daily{Date: b.Day, Open: b.Open, High: b.High, Low: b.Low, Close: b.Close, Volume: b.Volume})
		}
		return c.InPositionCurrencyDaily(wholeDaily(days), BarCurrency(source, key, rec), rec.Currency), nil
	}
	return []Daily{}, nil
}

type noteKey struct{ symbol, which string }

type chainNote struct {
	source, key string
	count       int
	err         error
}

func (c *Client) rememberNotes(rec Rec, which string, notes []chainNote) {
	c.hmu.Lock()
	c.notes[noteKey{TMXSymbol(rec.Symbol), which}] = append([]chainNote{}, notes...)
	c.hmu.Unlock()
}

func (c *Client) ClearNotes() {
	c.hmu.Lock()
	c.notes = map[noteKey][]chainNote{}
	c.hmu.Unlock()
}

func (c *Client) ChartReason(rec Rec, tf string) string {
	which := "daily"
	if _, ok := IntradaySeconds[tf]; ok {
		which = "hourly"
	}
	c.hmu.Lock()
	notes := append([]chainNote{}, c.notes[noteKey{TMXSymbol(rec.Symbol), which}]...)
	c.hmu.Unlock()
	var failed []string
	for _, n := range notes {
		if n.err != nil {
			line := SourceLabel(n.source) + " " + DescribeFailure(n.err)
			if !in(failed, line) {
				failed = append(failed, line)
			}
		}
	}
	if len(failed) > 0 {
		return strings.Join(failed, "; ") + "."
	}
	var names []string
	for _, cand := range HistoryCandidates(rec) {
		n := SourceLabel(cand.Source)
		if !in(names, n) {
			names = append(names, n)
		}
	}
	if len(names) == 0 {
		return "No price source covers this instrument."
	}
	joined := ""
	if len(names) <= 2 {
		joined = strings.Join(names, " or ")
	} else {
		joined = strings.Join(names[:len(names)-1], ", ") + " or " + names[len(names)-1]
	}
	return "No bars for this span from " + joined + "."
}

type answer struct {
	source, key string
	first       time.Time
	has         bool
}

func (c *Client) pickCovering(rec Rec, answers []answer, spanStart time.Time) (int, string) {
	slack := time.Duration(CoverageSlackDays) * 24 * time.Hour
	best := -1
	for i, a := range answers {
		if !a.has {
			continue
		}
		if !a.first.After(spanStart.Add(slack)) {
			c.rememberWinner(rec, a.source, a.key)
			return i, a.source
		}
		if best < 0 || a.first.Before(answers[best].first) {
			best = i
		}
	}
	if best >= 0 {
		c.rememberWinner(rec, answers[best].source, answers[best].key)
		return best, answers[best].source
	}
	return -1, ""
}

func dayTime(day string) time.Time {
	t, _ := py.ParseDate(day)
	return t
}

func (c *Client) FetchHistory(rec Rec, start, end string) ([]Daily, string) {
	spanStart := dayTime(start)
	var answers []answer
	var bars [][]Daily
	var notes []chainNote
	for _, cand := range c.OrderedCandidates(rec) {
		got, err := c.FetchDailyFrom(cand.Source, cand.Key, rec, start, end)
		if err != nil {
			got = []Daily{}
			notes = append(notes, chainNote{cand.Source, cand.Key, 0, err})
		} else {
			notes = append(notes, chainNote{cand.Source, cand.Key, len(got), nil})
		}
		a := answer{source: cand.Source, key: cand.Key, has: len(got) > 0}
		if a.has {
			a.first = dayTime(got[0].Date)
		}
		answers = append(answers, a)
		bars = append(bars, got)
		if a.has && !a.first.After(spanStart.Add(time.Duration(CoverageSlackDays)*24*time.Hour)) {
			break
		}
	}
	idx, source := c.pickCovering(rec, answers, spanStart)
	if idx < 0 {
		c.rememberNotes(rec, "daily", notes)
		return []Daily{}, ""
	}
	return bars[idx], source
}

func (c *Client) EnsureHistory(rec Rec, start, end string, now time.Time) []Daily {
	sym := TMXSymbol(rec.Symbol)
	if len(start) > 10 {
		start = start[:10]
	}
	if len(end) > 10 {
		end = end[:10]
	}
	if sym == "" || len(start) != 10 || len(end) != 10 {
		return []Daily{}
	}
	last := c.Store.HistoryFetch(sym)
	covered := last != nil && last.Start <= start
	fresh := false
	if last != nil {
		if age, ok := ageOf(last.FetchedAt, now); ok {
			fresh = age < HistoryStaleHours*time.Hour
		}
	}
	today := py.DateStr(now)
	needsRecent := end >= py.DateStr(now.AddDate(0, 0, -3))
	if !covered || (needsRecent && !fresh) {
		fetchFrom := start
		if covered && last.Start < start {
			fetchFrom = last.Start
		}
		bars, source := c.FetchHistory(rec, fetchFrom, today)
		if len(bars) > 0 {
			c.Store.UpsertPriceHistory(sym, bars, source)
			gotFrom := bars[0].Date
			coveredFrom := gotFrom
			if !dayTime(gotFrom).After(dayTime(fetchFrom).Add(time.Duration(CoverageSlackDays) * 24 * time.Hour)) {
				coveredFrom = fetchFrom
			}
			c.Store.MarkHistoryFetched(sym, coveredFrom, stamp(now))
		}
	}
	return c.Store.PriceHistory(sym, start, end)
}

func AggregateDaily(bars []Daily, tf string) []Daily {
	out := []Daily{}
	var cur *Daily
	for _, b := range bars {
		d, ok := py.ParseDate(b.Date)
		if !ok {
			continue
		}
		var key string
		if tf == "1w" {
			wd := (int(d.Weekday()) + 6) % 7
			key = py.DateStr(d.AddDate(0, 0, -wd))
		} else {
			key = py.DateStr(time.Date(d.Year(), d.Month(), 1, 0, 0, 0, 0, time.UTC))
		}
		if cur == nil || cur.Date != key {
			out = append(out, Daily{Date: key, Open: b.Open, High: b.High, Low: b.Low, Close: b.Close, Volume: b.Volume})
			cur = &out[len(out)-1]
			continue
		}
		cur.Close = b.Close
		if b.High != nil {
			if cur.High == nil || *b.High > *cur.High {
				cur.High = b.High
			}
		}
		if b.Low != nil {
			if cur.Low == nil || *b.Low < *cur.Low {
				cur.Low = b.Low
			}
		}
		if b.Volume != nil {
			cur.Volume = py.Ptr(py.Deref(cur.Volume, 0) + *b.Volume)
		}
	}
	return out
}

func minuteStamp(text string) (int64, string, int, int, bool) {
	if len(text) == 25 && text[4] == '-' && text[10] == 'T' && text[13] == ':' && text[22] == ':' && (text[19] == '+' || text[19] == '-') {
		h, e1 := strconv.Atoi(text[11:13])
		mi, e2 := strconv.Atoi(text[14:16])
		se, e3 := strconv.Atoi(text[17:19])
		oh, e4 := strconv.Atoi(text[20:22])
		om, e5 := strconv.Atoi(text[23:25])
		if e1 == nil && e2 == nil && e3 == nil && e4 == nil && e5 == nil {
			minute := h*60 + mi
			offset := oh*3600 + om
			if text[19] == '-' {
				offset = -offset
			}
			if day, ok := py.ParseDate(text[:10]); ok {
				return day.Unix() + int64(minute)*60 + int64(se) - int64(offset), text[:10], minute, offset, true
			}
		}
	}
	t, _, ok := py.ParseISO(text)
	if !ok {
		return 0, "", 0, 0, false
	}
	_, off := t.Zone()
	return t.Unix(), py.DateStr(t), t.Hour()*60 + t.Minute(), off, true
}

func ParseTMXMinutes(data map[string]any) []MinuteBar {
	d, _ := data["data"].(map[string]any)
	rows, _ := d["intraday"].([]any)
	out := []MinuteBar{}
	for _, raw := range rows {
		r, ok := raw.(map[string]any)
		if !ok || py.S(r["dateTime"]) == "" {
			continue
		}
		ts, day, minute, offset, ok := minuteStamp(py.S(r["dateTime"]))
		if !ok {
			continue
		}
		cl := numPtr(r["close"])
		if cl == nil || *cl <= 0 {
			continue
		}
		out = append(out, MinuteBar{Time: ts, Day: day, Minute: minute, Offset: offset, Open: numPtr(r["open"]), High: numPtr(r["high"]), Low: numPtr(r["low"]), Close: *cl, Volume: numPtr(r["volume"])})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Time < out[j].Time })
	return out
}

func AggregateSession(minutes []MinuteBar, bucketMinutes int) []Bar {
	type key struct {
		day string
		idx int
	}
	out := map[key]*Bar{}
	for _, m := range minutes {
		rel := m.Minute - SessionOpenMinutes
		if rel < 0 {
			rel = 0
		}
		idx := rel / bucketMinutes
		startMinute := SessionOpenMinutes + idx*bucketMinutes
		k := key{m.Day, idx}
		b, ok := out[k]
		if !ok {
			start := dayTs(m.Day) + int64(startMinute)*60 - int64(m.Offset)
			or := func(p *float64) *float64 {
				if p != nil {
					return py.Ptr(*p)
				}
				return py.Ptr(m.Close)
			}
			out[k] = &Bar{Time: start, Open: or(m.Open), High: or(m.High), Low: or(m.Low), Close: m.Close, Volume: py.Ptr(py.Deref(m.Volume, 0))}
			continue
		}
		b.Close = m.Close
		if m.High != nil && *m.High > *b.High {
			b.High = py.Ptr(*m.High)
		}
		if m.Low != nil && *m.Low < *b.Low {
			b.Low = py.Ptr(*m.Low)
		}
		b.Volume = py.Ptr(py.Deref(b.Volume, 0) + py.Deref(m.Volume, 0))
	}
	bars := make([]Bar, 0, len(out))
	for _, b := range out {
		bars = append(bars, *b)
	}
	sort.SliceStable(bars, func(i, j int) bool { return bars[i].Time < bars[j].Time })
	return bars
}

func (c *Client) FetchTMXMinutes(key, start, end string) []MinuteBar {
	type span struct{ a, b string }
	var chunks []span
	cur, ok1 := py.ParseDate(start)
	last, ok2 := py.ParseDate(end)
	if !ok1 || !ok2 {
		return []MinuteBar{}
	}
	for !cur.After(last) {
		first := time.Date(cur.Year(), cur.Month(), 1, 0, 0, 0, 0, time.UTC)
		nxt := first.AddDate(0, 0, 32)
		nxt = time.Date(nxt.Year(), nxt.Month(), 1, 0, 0, 0, 0, time.UTC).AddDate(0, 0, -1)
		stop := nxt
		if last.Before(stop) {
			stop = last
		}
		chunks = append(chunks, span{py.DateStr(cur), py.DateStr(stop)})
		cur = stop.AddDate(0, 0, 1)
	}
	results := parallelMap(chunks, 4, func(s span) []MinuteBar {
		data, err := c.PostJSON(TMXURL, map[string]any{"operationName": "getCompanyChart", "variables": map[string]any{"symbol": key, "from": s.a, "to": s.b}, "query": TMXChartQuery}, TMXHeaders)
		if err != nil {
			return nil
		}
		return ParseTMXMinutes(data)
	})
	out := []MinuteBar{}
	for _, bars := range results {
		out = append(out, bars...)
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Time < out[j].Time })
	return out
}

func (c *Client) IntradayReady(rec Rec, tf, start string, now time.Time) bool {
	sym := TMXSymbol(rec.Symbol)
	reach := IntradayReach(rec, now)
	if _, ok := IntradaySeconds[tf]; sym == "" || reach == "" || !ok {
		return true
	}
	if c.IntradayMissedRecently(sym, tf, now) {
		return true
	}
	startDay := start
	if len(startDay) > 10 {
		startDay = startDay[:10]
	}
	if reach > startDay {
		startDay = reach
	}
	startTs := dayTs(startDay)
	last := c.Store.BarFetchOf(sym, tf)
	return last != nil && last.StartTs <= startTs
}

func (c *Client) EnsureIntradayInBackground(rec Rec, tf, start, end string) {
	sym := TMXSymbol(rec.Symbol)
	c.pendingMu.Lock()
	if c.pending[sym] {
		c.pendingMu.Unlock()
		return
	}
	c.pending[sym] = true
	c.pendingMu.Unlock()
	go func() {
		defer func() {
			c.pendingMu.Lock()
			delete(c.pending, sym)
			c.pendingMu.Unlock()
		}()
		c.EnsureIntraday(rec, tf, start, end, c.now(), 1, true)
	}()
}

func AggregateHourly(bars []Bar, seconds int64) []Bar {
	out := map[int64]*Bar{}
	for _, b := range bars {
		k := b.Time / seconds * seconds
		hi := py.Deref(b.High, b.Close)
		lo := py.Deref(b.Low, b.Close)
		cur, ok := out[k]
		if !ok {
			out[k] = &Bar{Time: k, Open: py.Ptr(py.Deref(b.Open, b.Close)), High: py.Ptr(hi), Low: py.Ptr(lo), Close: b.Close, Volume: py.Ptr(py.Deref(b.Volume, 0))}
			continue
		}
		if hi > *cur.High {
			cur.High = py.Ptr(hi)
		}
		if lo < *cur.Low {
			cur.Low = py.Ptr(lo)
		}
		cur.Close = b.Close
		cur.Volume = py.Ptr(*cur.Volume + py.Deref(b.Volume, 0))
	}
	m := map[int64]Bar{}
	for k, b := range out {
		m[k] = *b
	}
	return sortedBars(m)
}

func SourceIntradayReach(source string, now time.Time) string {
	if source == "coinbase" {
		return CoinbaseExchangeStart
	}
	if days, ok := SourceIntradayDays[source]; ok && days > 0 {
		return py.DateStr(now.AddDate(0, 0, -days))
	}
	return ""
}

func IntradayReach(rec Rec, now time.Time) string {
	best := ""
	for _, cand := range HistoryCandidates(rec) {
		r := SourceIntradayReach(cand.Source, now)
		if r != "" && (best == "" || r < best) {
			best = r
		}
	}
	return best
}

func AvailableTimeframes(rec Rec, start string, now time.Time) []string {
	if len(HistoryCandidates(rec)) == 0 {
		return []string{}
	}
	out := []string{}
	reach := IntradayReach(rec, now)
	if len(start) > 10 {
		start = start[:10]
	}
	if reach != "" && start >= reach {
		out = append(out, "1h", "4h")
	}
	return append(out, "1d", "1w", "1M")
}

func (c *Client) FetchIntradayFrom(source, key string, rec Rec, startTs, endTs int64) (map[string][]Bar, error) {
	crypto := rec.Kind == "Crypto"
	switch source {
	case "tmx":
		start := py.DateStr(time.Unix(startTs, 0).UTC())
		end := py.DateStr(time.Unix(endTs, 0).UTC())
		first := c.TMXRemembered(key)
		minutes := c.FetchTMXMinutes(first, start, end)
		if len(minutes) == 0 && key != "" && !strings.HasPrefix(key, "^") {
			if alt := c.TMXResolve(key); alt != "" && alt != first {
				minutes = c.FetchTMXMinutes(alt, start, end)
			}
		}
		if len(minutes) == 0 {
			return map[string][]Bar{}, nil
		}
		return map[string][]Bar{"1h": AggregateSession(minutes, 60), "4h": AggregateSession(minutes, 240)}, nil
	case "coinbase":
		if c.CoinbaseMarket(key, c.now()) == "" {
			return map[string][]Bar{}, nil
		}
		hourly := c.InPositionCurrencyBars(c.FetchCoinbaseCandles(key, 3600, startTs, endTs), BarCurrency(source, key, rec), rec.Currency)
		if len(hourly) == 0 {
			return map[string][]Bar{}, nil
		}
		return map[string][]Bar{"1h": hourly, "4h": AggregateHourly(hourly, 14400)}, nil
	case "yahoo":
		mins, err := c.FetchYahoo(key, startTs, endTs, "60m")
		if err != nil {
			return nil, err
		}
		whole := wholeMinutes(mins)
		if crypto {
			bars := make([]Bar, 0, len(whole))
			for _, m := range whole {
				bars = append(bars, m.bar())
			}
			hourly := c.InPositionCurrencyBars(bars, BarCurrency(source, key, rec), rec.Currency)
			if len(hourly) == 0 {
				return map[string][]Bar{}, nil
			}
			return map[string][]Bar{"1h": hourly, "4h": AggregateHourly(hourly, 14400)}, nil
		}
		hourly := c.inPositionCurrencyMinutes(whole, BarCurrency(source, key, rec), rec.Currency)
		if len(hourly) == 0 {
			return map[string][]Bar{}, nil
		}
		return map[string][]Bar{"1h": AggregateSession(hourly, 60), "4h": AggregateSession(hourly, 240)}, nil
	}
	return map[string][]Bar{}, nil
}

func (c *Client) inPositionCurrencyMinutes(bars []MinuteBar, barCurrency, currency string) []MinuteBar {
	quote := strings.ToUpper(barCurrency)
	ccy := strings.ToUpper(currency)
	if ccy == "" {
		ccy = "CAD"
	}
	if quote == ccy {
		return append([]MinuteBar{}, bars...)
	}
	if !(quote == "USD" && ccy == "CAD") {
		return []MinuteBar{}
	}
	fx := c.Store.FXRates()
	out := []MinuteBar{}
	for _, b := range bars {
		rate := rateOnOrBefore(fx, py.DateStr(time.Unix(b.Time, 0).UTC()), 7)
		if rate == nil {
			continue
		}
		nb := b
		nb.Open, nb.High, nb.Low, nb.Close = scale(b.Open, *rate), scale(b.High, *rate), scale(b.Low, *rate), b.Close**rate
		out = append(out, nb)
	}
	return out
}

func missKey(symbol, tf string) string { return "bars_miss:" + TMXSymbol(symbol) + "|" + tf }

func (c *Client) RecordIntradayMiss(symbol, tf string, now time.Time) {
	c.Store.SetMeta(missKey(symbol, tf), stamp(now))
}

func (c *Client) IntradayMissedRecently(symbol, tf string, now time.Time) bool {
	v := c.Store.GetMeta(missKey(symbol, tf))
	if v == "" {
		return false
	}
	age, ok := ageOf(v, now)
	return ok && age < IntradayRetryMinutes*time.Minute
}

func (c *Client) OfferedTimeframes(rec Rec, start string, now time.Time) []string {
	out := AvailableTimeframes(rec, start, now)
	sym := TMXSymbol(rec.Symbol)
	kept := []string{}
	for _, tf := range out {
		if _, intraday := IntradaySeconds[tf]; !intraday || !c.IntradayMissedRecently(sym, tf, now) || len(c.Store.PriceBars(sym, tf, 0, 1<<40)) > 0 {
			kept = append(kept, tf)
		}
	}
	return kept
}

func (c *Client) FetchIntraday(rec Rec, startTs, endTs int64, onDemand bool) (map[string][]Bar, string) {
	startDay := py.DateStr(time.Unix(startTs, 0).UTC())
	spanStart := time.Unix(startTs, 0).UTC()
	var answers []answer
	var byTFs []map[string][]Bar
	var notes []chainNote
	slack := time.Duration(CoverageSlackDays) * 24 * time.Hour
	for _, cand := range c.OrderedCandidates(rec) {
		reach := SourceIntradayReach(cand.Source, c.now())
		if reach == "" || startDay < reach || (!onDemand && in(OnDemandOnlySources, cand.Source)) {
			continue
		}
		byTF, err := c.FetchIntradayFrom(cand.Source, cand.Key, rec, startTs, endTs)
		if err != nil {
			byTF = map[string][]Bar{}
			notes = append(notes, chainNote{cand.Source, cand.Key, 0, err})
		} else {
			notes = append(notes, chainNote{cand.Source, cand.Key, len(byTF["1h"]), nil})
		}
		a := answer{source: cand.Source, key: cand.Key, has: len(byTF["1h"]) > 0}
		if a.has {
			a.first = time.Unix(byTF["1h"][0].Time, 0).UTC()
		}
		answers = append(answers, a)
		byTFs = append(byTFs, byTF)
		if a.has && !a.first.After(spanStart.Add(slack)) {
			break
		}
	}
	idx, source := c.pickCovering(rec, answers, spanStart)
	if idx < 0 {
		c.rememberNotes(rec, "hourly", notes)
		return map[string][]Bar{}, ""
	}
	return byTFs[idx], source
}

func (c *Client) EnsureIntraday(rec Rec, tf, start, end string, now time.Time, maxAgeHours float64, onDemand bool) []Bar {
	sym := TMXSymbol(rec.Symbol)
	reach := IntradayReach(rec, now)
	if _, ok := IntradaySeconds[tf]; sym == "" || reach == "" || !ok {
		return []Bar{}
	}
	if len(start) > 10 {
		start = start[:10]
	}
	if len(end) > 10 {
		end = end[:10]
	}
	startDay := start
	if reach > startDay {
		startDay = reach
	}
	startTs := dayTs(startDay)
	endTs := dayTs(end) + 86400
	if now.Unix() < endTs {
		endTs = now.Unix()
	}
	last := c.Store.BarFetchOf(sym, tf)
	covered := last != nil && last.StartTs <= startTs
	fresh := false
	if last != nil {
		if age, ok := ageOf(last.FetchedAt, now); ok {
			fresh = age < time.Duration(maxAgeHours*float64(time.Hour))
		}
	}
	needsRecent := endTs >= now.Unix()-3*86400
	var fetchFrom *int64
	if !covered {
		fetchFrom = py.PtrInt64(startTs)
	} else if needsRecent && !fresh {
		newest := c.Store.LastBarTime(sym, tf)
		base := startTs
		if newest != nil {
			base = *newest
		}
		from := base - 2*86400
		if startTs > from {
			from = startTs
		}
		fetchFrom = py.PtrInt64(from)
	}
	if fetchFrom != nil {
		byTF, source := c.FetchIntraday(rec, *fetchFrom, now.Unix(), onDemand)
		for _, k := range []string{"1h", "4h"} {
			if len(byTF[k]) == 0 {
				c.RecordIntradayMiss(sym, k, now)
			}
		}
		for _, k := range sortedTFs(byTF) {
			bars := byTF[k]
			if len(bars) == 0 {
				continue
			}
			c.Store.UpsertPriceBars(sym, k, bars, source)
			coveredFrom := bars[0].Time
			if bars[0].Time <= *fetchFrom+CoverageSlackDays*86400 {
				coveredFrom = *fetchFrom
			}
			if last != nil && last.StartTs < coveredFrom {
				coveredFrom = last.StartTs
			}
			c.Store.MarkBarsFetched(sym, k, coveredFrom, stamp(now))
		}
	}
	return c.Store.PriceBars(sym, tf, startTs, endTs)
}

func sortedTFs(m map[string][]Bar) []string {
	out := []string{}
	for _, k := range []string{"1h", "4h"} {
		if _, ok := m[k]; ok {
			out = append(out, k)
		}
	}
	for k := range m {
		if !in(out, k) {
			out = append(out, k)
		}
	}
	return out
}

type archiveItem struct {
	rank int
	sym  string
	rec  Rec
}

func sortArchive(todo []archiveItem) {
	sort.SliceStable(todo, func(i, j int) bool {
		if todo[i].rank != todo[j].rank {
			return todo[i].rank < todo[j].rank
		}
		return todo[i].sym < todo[j].sym
	})
}

func (c *Client) ArchiveIntraday(recs []Rec, now time.Time, limit int) []string {
	var todo []archiveItem
	for _, rec := range recs {
		sym := TMXSymbol(rec.Symbol)
		if sym == "" || IntradayReach(rec, now) == "" {
			continue
		}
		last := c.Store.BarFetchOf(sym, "1h")
		if last == nil {
			todo = append(todo, archiveItem{0, sym, rec})
			continue
		}
		age, ok := ageOf(last.FetchedAt, now)
		if !ok || age > ArchiveTopupHours*time.Hour {
			todo = append(todo, archiveItem{1, sym, rec})
		}
	}
	sortArchive(todo)
	if len(todo) > limit {
		todo = todo[:limit]
	}
	done := []string{}
	for _, item := range todo {
		start := item.rec.Start
		if start == "" {
			start = py.DateStr(now)
		}
		c.EnsureIntraday(item.rec, "1h", start, py.DateStr(now), now, ArchiveTopupHours, false)
		done = append(done, item.sym)
	}
	return done
}

func (c *Client) ArchiveDaily(recs []Rec, now time.Time, limit int) []string {
	var todo []archiveItem
	for _, rec := range recs {
		src := HistorySource(rec)
		sym := TMXSymbol(rec.Symbol)
		if src == nil || !in(ShortDailySources, src.Source) || sym == "" {
			continue
		}
		last := c.Store.HistoryFetch(sym)
		if last == nil {
			todo = append(todo, archiveItem{0, sym, rec})
			continue
		}
		age, ok := ageOf(last.FetchedAt, now)
		if !ok || age > ArchiveTopupHours*time.Hour {
			todo = append(todo, archiveItem{1, sym, rec})
		}
	}
	sortArchive(todo)
	if len(todo) > limit {
		todo = todo[:limit]
	}
	done := []string{}
	for _, item := range todo {
		start := item.rec.Start
		if start == "" {
			start = py.DateStr(now)
		}
		c.EnsureHistory(item.rec, start, py.DateStr(now), now)
		done = append(done, item.sym)
	}
	return done
}

func (c *Client) EnsureBars(rec Rec, tf, start, end string, now time.Time) ([]Daily, []Bar) {
	if _, ok := IntradaySeconds[tf]; ok {
		return nil, c.EnsureIntraday(rec, tf, start, end, now, 1, true)
	}
	daily := c.EnsureHistory(rec, start, end, now)
	if tf == "1d" {
		return daily, nil
	}
	return AggregateDaily(daily, tf), nil
}

func (c *Client) TMXLookupDaily(key string, fn func(form string) ([]Daily, error)) ([]Daily, error) {
	return c.tmxLookupDaily(key, fn)
}

func (c *Client) YahooPaced(call func() (*browserhttp.Response, error)) (*browserhttp.Response, bool) {
	c.yahooMu.Lock()
	defer c.yahooMu.Unlock()
	if time.Now().Before(c.yahooBackoffUntil) {
		return nil, false
	}
	if wait := c.yahooNextAt.Sub(time.Now()); wait > 0 {
		time.Sleep(wait)
	}
	c.yahooNextAt = time.Now().Add(time.Duration(YahooMinIntervalSec * float64(time.Second)))
	resp, err := call()
	if err != nil {
		return nil, false
	}
	if resp.Status == 429 {
		c.yahooBackoffUntil = time.Now().Add(YahooBackoffSec * time.Second)
		return nil, false
	}
	return resp, true
}
