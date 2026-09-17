package market

import (
	"encoding/json"
	"math"
	"net/url"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const (
	BocURL              = "https://www.bankofcanada.ca/valet/observations/FXUSDCAD/json"
	FredURL             = "https://fred.stlouisfed.org/graph/fredgraph.csv?id=SP500"
	StooqURL            = "https://stooq.com/q/d/l/?s=^spx&i=d"
	TMXURL              = "https://app-money.tmx.com/graphql"
	TMXQuoteQuery       = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name exchangeName price priceChange percentChange prevClose currency dividendFrequency dividendYield dividendAmount exDividendDate } }"
	TMXDividendsQuery   = "query getDividendsForSymbol($symbol: String!, $page: Int, $batch: Int) { dividends: getDividendsForSymbol(symbol: $symbol, page: $page, batch: $batch) { dividends { exDate payableDate amount currency } } }"
	TMXHistoryQuery     = "query getTimeSeriesData($symbol: String!, $freq: String, $interval: Int, $start: String, $end: String) { getTimeSeriesData(symbol: $symbol, freq: $freq, interval: $interval, start: $start, end: $end) { dateTime open high low close volume } }"
	TMXChartQuery       = "query getCompanyChart($symbol: String!, $from: String!, $to: String!) { intraday: getChartDataBySymbol(symbol: $symbol, fromDate: $from, toDate: $to) { dateTime open high low close volume } }"
	QuoteRefreshMinutes = 1
	MarketCheckMinutes  = 60
	TMXBatch            = 24
	QuoteStaleHours     = 20
	CoinbaseURL         = "https://api.coinbase.com/v2/prices/%s/spot"
	CboeCaURL           = "https://www-api.cboe.com/ca/equities/securities-1/%s/quote/"
	CboeOptionsURL      = "https://cdn.cboe.com/api/global/delayed_quotes/options/%s.json"
	RecordStaleHours    = QuoteStaleHours
	MarketAttemptHours  = 6
	FXStart             = "2016-01-01"
	StaleDays           = 4
	TSXStart            = "2016-01-01"
	TMXResolveRetryDays = 1
	PeekSeconds         = 60
)

var TMXHeaders = map[string]string{"locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}

var Benchmarks = map[string]string{"SP500": "S&P 500", "TSX": "S&P/TSX", "TSX60": "TSX 60"}
var TMXIndices = map[string]string{"TSX": "^TSX", "TSX60": "^TX60"}
var tmxIndexOrder = []string{"TSX", "TSX60"}

var TMXForms = map[string][]string{"CAD": {"", ":CNX", ":AQL"}, "USD": {":US"}}
var TMXVenueOfForm = map[string][]string{"": {"TORONTO STOCK EXCHANGE", "TSX VENTURE"}, ":CNX": {"CANADIAN SECURITIES EXCHANGE"}, ":AQL": {"CBOE", "NEO"}, ":US": {"NYSE", "NASDAQ", "NEW YORK"}}

var TMXExchangeNames = []struct{ Mark, Venue string }{{"VENTURE", "TSX-V"}, {"TORONTO", "TSX"}, {"CANADIAN SECURITIES", "CSE"}, {"CBOE", "Cboe Canada"}, {"NEO", "Cboe Canada"}, {"NASDAQ", "NASDAQ"}, {"NYSE", "NYSE"}, {"NEW YORK", "NYSE"}}

func numPtr(v any) *float64 {
	f, ok := py.NumOK(v)
	if !ok {
		return nil
	}
	return &f
}

func ParseBocJSON(text string) map[string]float64 {
	out := map[string]float64{}
	var data struct {
		Observations []map[string]any `json:"observations"`
	}
	if err := json.Unmarshal([]byte(text), &data); err != nil {
		return out
	}
	for _, ob := range data.Observations {
		d := py.S(ob["d"])
		if len(d) > 10 {
			d = d[:10]
		}
		cell, _ := ob["FXUSDCAD"].(map[string]any)
		if cell == nil {
			continue
		}
		v, ok := py.NumOK(cell["v"])
		if !ok {
			continue
		}
		if len(d) == 10 && v > 0 {
			out[d] = v
		}
	}
	return out
}

func ParseFredCSV(text string) map[string]float64 {
	out := map[string]float64{}
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimRight(line, "\r")
		parts := strings.Split(line, ",")
		if len(parts) < 2 {
			continue
		}
		d := strings.TrimSpace(parts[0])
		raw := strings.TrimSpace(parts[1])
		if len(d) != 10 || d[4] != '-' || d[7] != '-' || raw == "" || raw == "." {
			continue
		}
		px, err := strconv.ParseFloat(raw, 64)
		if err != nil {
			continue
		}
		if px > 0 {
			out[d] = px
		}
	}
	return out
}

func ParseStooqCSV(text string) map[string]float64 {
	out := map[string]float64{}
	lines := strings.Split(text, "\n")
	if len(lines) > 0 {
		lines = lines[1:]
	}
	for _, line := range lines {
		line = strings.TrimRight(line, "\r")
		parts := strings.Split(line, ",")
		if len(parts) < 5 {
			continue
		}
		d := strings.TrimSpace(parts[0])
		px, err := strconv.ParseFloat(strings.TrimSpace(parts[4]), 64)
		if err != nil {
			continue
		}
		if len(d) == 10 && px > 0 {
			out[d] = px
		}
	}
	return out
}

func weekBefore(day string) string {
	t, ok := py.ParseDate(day)
	if !ok {
		return day
	}
	return py.DateStr(t.AddDate(0, 0, -7))
}

func (c *Client) RefreshFX() int {
	last := c.Store.FXLastDate()
	start := FXStart
	if last != "" {
		start = weekBefore(last)
	}
	text, err := c.GetText(BocURL+"?start_date="+start, nil)
	if err != nil {
		return 0
	}
	return c.Store.UpsertFXRates(ParseBocJSON(text))
}

func (c *Client) RefreshBenchmark() int {
	mapping := map[string]float64{}
	if text, err := c.GetText(FredURL, nil); err == nil {
		mapping = ParseFredCSV(text)
	}
	if len(mapping) == 0 {
		if text, err := c.GetText(StooqURL, nil); err == nil {
			mapping = ParseStooqCSV(text)
		}
	}
	if len(mapping) == 0 {
		return 0
	}
	if last := c.Store.BenchmarkLastDate(store.BenchmarkSymbol); last != "" {
		cutoff := weekBefore(last)
		kept := map[string]float64{}
		for d, v := range mapping {
			if d >= cutoff {
				kept[d] = v
			}
		}
		mapping = kept
	}
	return c.Store.UpsertBenchmarkPrices(mapping, store.BenchmarkSymbol)
}

func (c *Client) RefreshTMXIndex(key string) int {
	last := c.Store.BenchmarkLastDate(key)
	start := TSXStart
	if last != "" {
		start = weekBefore(last)
	}
	data, err := c.PostJSON(TMXURL, map[string]any{"operationName": "getTimeSeriesData", "variables": map[string]any{"symbol": TMXIndices[key], "freq": "day", "interval": 1, "start": start, "end": py.DateStr(time.Now())}, "query": TMXHistoryQuery}, TMXHeaders)
	if err != nil {
		return 0
	}
	mapping := map[string]float64{}
	for _, b := range ParseTMXHistory(data) {
		if b.Close != 0 {
			mapping[b.Date] = b.Close
		}
	}
	if len(mapping) == 0 {
		return 0
	}
	return c.Store.UpsertBenchmarkPrices(mapping, key)
}

func (c *Client) RefreshTSX() int {
	n := 0
	for _, key := range tmxIndexOrder {
		n += c.RefreshTMXIndex(key)
	}
	return n
}

func (c *Client) TMXRemembered(key string) string {
	if key == "" || strings.HasPrefix(key, "^") {
		return key
	}
	v := c.Store.GetMeta("tmx_form:" + TMXBare(key))
	if strings.HasPrefix(v, "@") {
		return TMXBare(key) + v[1:]
	}
	return key
}

func (c *Client) TMXResolve(key string) string {
	if key == "" || strings.HasPrefix(key, "^") {
		return key
	}
	bare := TMXBare(key)
	suffix := key[len(bare):]
	forms := TMXForms["CAD"]
	if suffix == ":US" {
		forms = TMXForms["USD"]
	}
	ordered := []string{}
	if in(forms, suffix) {
		ordered = append(ordered, suffix)
		for _, f := range forms {
			if f != suffix {
				ordered = append(ordered, f)
			}
		}
	} else {
		ordered = append(ordered, forms...)
	}
	metaKey := "tmx_form:" + bare
	v := c.Store.GetMeta(metaKey)
	if strings.HasPrefix(v, "@") {
		return bare + v[1:]
	}
	today := c.now()
	if strings.HasPrefix(v, "none@") && v[5:] > py.DateStr(today.AddDate(0, 0, -TMXResolveRetryDays)) {
		return ""
	}
	for _, form := range ordered {
		cand := bare + form
		q := map[string]any{}
		data, err := c.PostJSON(TMXURL, map[string]any{"operationName": "getQuoteBySymbol", "variables": map[string]any{"symbol": cand, "locale": "en"}, "query": TMXQuoteQuery}, TMXHeaders)
		if err == nil {
			if d, ok := data["data"].(map[string]any); ok {
				if qq, ok := d["getQuoteBySymbol"].(map[string]any); ok {
					q = qq
				}
			}
		}
		venue := strings.ToUpper(py.S(q["exchangeName"]))
		if venue != "" {
			for _, mark := range TMXVenueOfForm[form] {
				if strings.Contains(venue, mark) {
					c.Store.SetMeta(metaKey, "@"+form)
					return cand
				}
			}
		}
	}
	c.Store.SetMeta(metaKey, "none@"+py.DateStr(today))
	return ""
}

func (c *Client) tmxLookupQuote(key string, fn func(form string) *store.Quote) (*store.Quote, string) {
	first := c.TMXRemembered(key)
	r := fn(first)
	if r != nil || key == "" || strings.HasPrefix(key, "^") {
		return r, first
	}
	alt := c.TMXResolve(key)
	if alt != "" && alt != first {
		return fn(alt), alt
	}
	return r, first
}

func TMXVenue(name string) string {
	up := strings.ToUpper(name)
	for _, e := range TMXExchangeNames {
		if strings.Contains(up, e.Mark) {
			return e.Venue
		}
	}
	return ""
}

type Listing struct {
	Symbol   string `json:"symbol"`
	Name     string `json:"name"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
}

func (c *Client) TMXListing(symbol string) *Listing {
	bare := TMXBare(TMXSymbol(symbol))
	if bare == "" || strings.Contains(bare, " ") {
		return nil
	}
	form := c.TMXResolve(bare)
	if form == "" {
		return nil
	}
	data, err := c.PostJSON(TMXURL, map[string]any{"operationName": "getQuoteBySymbol", "variables": map[string]any{"symbol": form, "locale": "en"}, "query": TMXQuoteQuery}, TMXHeaders)
	if err != nil {
		return nil
	}
	q := ParseTMXQuote(data)
	if q == nil {
		return nil
	}
	venue := TMXVenue(q.Exchange)
	if venue == "" {
		return nil
	}
	name := q.Name
	if name == "" {
		name = bare
	}
	ccy := q.Currency
	if ccy == "" {
		ccy = "CAD"
		if venue == "NYSE" || venue == "NASDAQ" {
			ccy = "USD"
		}
	}
	return &Listing{Symbol: bare, Name: name, Exchange: venue, Currency: ccy}
}

func ParseTMXQuote(data map[string]any) *store.Quote {
	d, _ := data["data"].(map[string]any)
	q, _ := d["getQuoteBySymbol"].(map[string]any)
	if len(q) == 0 {
		return nil
	}
	ex := py.S(q["exDividendDate"])
	if len(ex) > 10 {
		ex = ex[:10]
	}
	return &store.Quote{
		Price:             numPtr(q["price"]),
		PriceChange:       numPtr(q["priceChange"]),
		PercentChange:     numPtr(q["percentChange"]),
		PrevClose:         numPtr(q["prevClose"]),
		Currency:          py.S(q["currency"]),
		DividendAmount:    numPtr(q["dividendAmount"]),
		DividendFrequency: py.S(q["dividendFrequency"]),
		ExDividendDate:    ex,
		Name:              py.S(q["name"]),
		Exchange:          py.S(q["exchangeName"]),
	}
}

func ParseTMXDividends(data map[string]any) []store.Distribution {
	d, _ := data["data"].(map[string]any)
	block, _ := d["dividends"].(map[string]any)
	rows, _ := block["dividends"].([]any)
	out := []store.Distribution{}
	for _, raw := range rows {
		r, ok := raw.(map[string]any)
		if !ok {
			continue
		}
		ex := py.S(r["exDate"])
		if len(ex) > 10 {
			ex = ex[:10]
		}
		amt, ok := py.NumOK(r["amount"])
		if !ok {
			continue
		}
		if len(ex) == 10 && amt > 0 {
			pay := py.S(r["payableDate"])
			if len(pay) > 10 {
				pay = pay[:10]
			}
			out = append(out, store.Distribution{ExDate: ex, PayDate: pay, Amount: amt, Currency: py.S(r["currency"])})
		}
	}
	return out
}

func (c *Client) tmxQuote(tmxSym string) *store.Quote {
	data, err := c.PostJSON(TMXURL, map[string]any{"operationName": "getQuoteBySymbol", "variables": map[string]any{"symbol": tmxSym, "locale": "en"}, "query": TMXQuoteQuery}, TMXHeaders)
	if err != nil {
		return nil
	}
	return ParseTMXQuote(data)
}

func (c *Client) FetchTMX(symbol, exchange string) (*store.Quote, []store.Distribution) {
	sym := TMXRecordSymbol(symbol, exchange)
	if sym == "" {
		return nil, nil
	}
	quote, form := c.tmxLookupQuote(sym, c.tmxQuote)
	divs := []store.Distribution{}
	data, err := c.PostJSON(TMXURL, map[string]any{"operationName": "getDividendsForSymbol", "variables": map[string]any{"symbol": form, "page": 1, "batch": TMXBatch}, "query": TMXDividendsQuery}, TMXHeaders)
	if err == nil {
		divs = ParseTMXDividends(data)
	}
	return quote, divs
}

func (c *Client) FetchTMXQuote(tmxSym string) *store.Quote {
	q, _ := c.tmxLookupQuote(tmxSym, c.tmxQuote)
	return q
}

var occWordy = regexp.MustCompile(`^([A-Z][A-Z0-9.]{0,9}) (\d{1,2})([A-Z]{3})(\d{2}) (\d+(?:\.\d+)?) (CALL|PUT|C|P)$`)
var occCompact = regexp.MustCompile(`^([A-Z][A-Z0-9.]{0,9}) (\d{6}[CP]\d{8})$`)
var occRootRE = regexp.MustCompile(`^([A-Z][A-Z0-9.]{0,9})\d{6}[CP]\d{8}$`)
var spaceRunRE = regexp.MustCompile(`[` + py.SpaceClass + `]+`)
var months = []string{"JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"}

func OccCode(symbol string) string {
	u := spaceRunRE.ReplaceAllString(strings.ToUpper(py.Strip(symbol)), " ")
	if m := occCompact.FindStringSubmatch(u); m != nil {
		return m[1] + m[2]
	}
	m := occWordy.FindStringSubmatch(u)
	if m == nil {
		return ""
	}
	mon := -1
	for i, name := range months {
		if name == m[3] {
			mon = i + 1
		}
	}
	if mon < 0 {
		return ""
	}
	day, _ := strconv.Atoi(m[2])
	strike, _ := strconv.ParseFloat(m[5], 64)
	return m[1] + m[4] + pad2(mon) + pad2(day) + m[6][:1] + pad8(int(math.RoundToEven(strike*1000)))
}

func pad2(n int) string {
	s := strconv.Itoa(n)
	if len(s) < 2 {
		s = "0" + s
	}
	return s
}

func pad8(n int) string {
	s := strconv.Itoa(n)
	for len(s) < 8 {
		s = "0" + s
	}
	return s
}

func OccRoot(code string) string {
	if m := occRootRE.FindStringSubmatch(code); m != nil {
		return m[1]
	}
	return ""
}

func ParseYahooQuote(text string) *store.Quote {
	var d map[string]any
	if text == "" {
		text = "{}"
	}
	if err := json.Unmarshal([]byte(text), &d); err != nil {
		return nil
	}
	chart, _ := d["chart"].(map[string]any)
	results, _ := chart["result"].([]any)
	if len(results) == 0 {
		return nil
	}
	first, _ := results[0].(map[string]any)
	meta, ok := first["meta"].(map[string]any)
	if !ok || meta["regularMarketPrice"] == nil {
		return nil
	}
	last := numPtr(meta["regularMarketPrice"])
	prev := numPtr(meta["chartPreviousClose"])
	if prev == nil {
		prev = numPtr(meta["previousClose"])
	}
	var change, pct *float64
	if last != nil && prev != nil && *prev != 0 {
		change = py.Ptr(*last - *prev)
		pct = py.Ptr(*change / *prev * 100.0)
	}
	name := py.S(meta["shortName"])
	if name == "" {
		name = py.S(meta["longName"])
	}
	return &store.Quote{Price: last, PriceChange: change, PercentChange: pct, PrevClose: prev, Currency: py.S(meta["currency"]), Name: name, Exchange: py.S(meta["exchangeName"])}
}

func (c *Client) FetchYahooQuote(code string) *store.Quote {
	text, err := c.yahooGet("https://query1.finance.yahoo.com/v8/finance/chart/" + url.PathEscape(code) + "?range=1d&interval=1d")
	if err != nil {
		return nil
	}
	return ParseYahooQuote(text)
}

func QuoteSource(rec Rec) (string, string, bool) {
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
		return "", "", false
	}
	if kind == "Instrument" {
		if rec.Yahoo != "" {
			return "yahoo_quote", rec.Yahoo, true
		}
		return "", "", false
	}
	if kind == "Crypto" {
		return "coinbase", sym + "-" + ccy, true
	}
	if kind == "Options" {
		code := OccCode(rec.Symbol)
		if code != "" && ccy == "USD" {
			return "cboe_options", code, true
		}
		return "", "", false
	}
	if kind != "Shares" {
		return "", "", false
	}
	if in(CboeCanadaExchanges, strings.ToUpper(strings.TrimSpace(rec.Exchange))) {
		return "cboe_ca", sym, true
	}
	if f := TMXForm(rec.Exchange, rec.Currency); f != nil && *f == ":US" {
		forms := YahooFormsFor(rec)
		if len(forms) > 0 {
			return "yahoo_quote", forms[0], true
		}
		return "", "", false
	}
	if q := TMXQuoteSymbol(rec.Symbol, rec.Exchange, rec.Currency); q != "" {
		return "tmx", q, true
	}
	return "", "", false
}

func ParseCoinbase(text, pair string) *store.Quote {
	var d struct {
		Data map[string]any `json:"data"`
	}
	if text == "" {
		text = "{}"
	}
	if err := json.Unmarshal([]byte(text), &d); err != nil {
		return nil
	}
	px := numPtr(d.Data["amount"])
	if px == nil || *px <= 0 {
		return nil
	}
	ccy := py.S(d.Data["currency"])
	if ccy == "" {
		parts := strings.Split(pair, "-")
		ccy = parts[len(parts)-1]
	}
	return &store.Quote{Price: px, Currency: ccy}
}

func ParseCboeCaQuote(text string) *store.Quote {
	var d struct {
		Data map[string]any `json:"data"`
	}
	if text == "" {
		text = "{}"
	}
	if err := json.Unmarshal([]byte(text), &d); err != nil {
		return nil
	}
	last := numPtr(d.Data["last"])
	prev := numPtr(d.Data["prev_close"])
	px := prev
	if last != nil && *last > 0 {
		px = last
	}
	if px == nil || *px <= 0 {
		return nil
	}
	return &store.Quote{Price: px, PriceChange: numPtr(d.Data["change"]), PercentChange: numPtr(d.Data["change_pct"]), PrevClose: prev, Currency: "CAD", Name: py.S(d.Data["company_name"])}
}

type OptionChain map[string]map[string]any

func ParseCboeOptions(text string) OptionChain {
	var d struct {
		Data struct {
			Options []any `json:"options"`
		} `json:"data"`
	}
	if text == "" {
		text = "{}"
	}
	out := OptionChain{}
	if err := json.Unmarshal([]byte(text), &d); err != nil {
		return out
	}
	for _, raw := range d.Data.Options {
		if o, ok := raw.(map[string]any); ok {
			out[py.S(o["option"])] = o
		}
	}
	return out
}

func OptionMark(row map[string]any) *store.Quote {
	if row == nil {
		return nil
	}
	bid := py.Num(row["bid"], 0)
	ask := py.Num(row["ask"], 0)
	prev := numPtr(row["prev_day_close"])
	var px *float64
	if bid > 0 && ask > 0 {
		px = py.Ptr((bid + ask) / 2)
	} else {
		px = numPtr(row["last_trade_price"])
		if px == nil || *px == 0 {
			px = prev
		}
	}
	if px == nil || *px <= 0 {
		return nil
	}
	q := &store.Quote{Price: px, PrevClose: prev, Currency: "USD"}
	if prev != nil && *prev != 0 {
		q.PriceChange = py.Ptr(*px - *prev)
		q.PercentChange = py.Ptr((*px / *prev - 1) * 100)
	}
	return q
}

func (c *Client) CoinbasePrevClose(pair string, now time.Time) *float64 {
	pair = strings.ToUpper(strings.TrimSpace(pair))
	today := py.DateStr(now)
	metaKey := "coinbase_prev:" + pair
	v := c.Store.GetMeta(metaKey)
	if strings.HasPrefix(v, today+"@") {
		f, err := strconv.ParseFloat(v[len(today)+1:], 64)
		if err != nil {
			return nil
		}
		return &f
	}
	base, ccy, _ := strings.Cut(pair, "-")
	products := []string{pair}
	if ccy != "USD" {
		products = append(products, base+"-USD")
	}
	var prev *float64
	for _, product := range products {
		if c.CoinbaseMarket(product, now) == "" {
			continue
		}
		start := now.Add(-4 * 24 * time.Hour).Unix()
		bars := c.FetchCoinbaseCandles(product, 86400, start, now.Unix())
		parts := strings.Split(product, "-")
		bars = c.InPositionCurrencyBars(bars, parts[len(parts)-1], ccy)
		var done []store.Bar
		for _, b := range bars {
			if py.DateStr(time.Unix(b.Time, 0).UTC()) < today {
				done = append(done, b)
			}
		}
		if len(done) > 0 {
			prev = py.Ptr(done[len(done)-1].Close)
			break
		}
	}
	if prev != nil && *prev != 0 {
		c.Store.SetMeta(metaKey, today+"@"+py.Repr(*prev))
	}
	return prev
}

func (c *Client) FetchCoinbaseSpot(pair string, now time.Time) *store.Quote {
	text, err := c.GetText(strings.Replace(CoinbaseURL, "%s", pair, 1), nil)
	if err != nil {
		return nil
	}
	rec := ParseCoinbase(text, pair)
	if rec == nil {
		return nil
	}
	prev := c.CoinbasePrevClose(pair, now)
	if prev != nil && *prev != 0 {
		rec.PrevClose = prev
		rec.PriceChange = py.Ptr(*rec.Price - *prev)
		rec.PercentChange = py.Ptr((*rec.Price - *prev) / *prev * 100.0)
	}
	return rec
}

func (c *Client) FetchCboeCaQuote(sym string) *store.Quote {
	text, err := c.GetText(strings.Replace(CboeCaURL, "%s", sym, 1), nil)
	if err != nil {
		return nil
	}
	return ParseCboeCaQuote(text)
}

func (c *Client) FetchCboeOptionChain(root string) OptionChain {
	text, err := c.GetText(strings.Replace(CboeOptionsURL, "%s", root, 1), nil)
	if err != nil {
		return OptionChain{}
	}
	return ParseCboeOptions(text)
}

type Needed struct {
	Symbol string
	Source string
	Key    string
}

func (c *Client) QuoteSymbolsNeedingRefresh(symbols []Rec, now time.Time, maxAgeMinutes float64) []Needed {
	fetched := c.Store.QuoteFetchedAt()
	out := []Needed{}
	seen := map[string]bool{}
	for _, rec := range symbols {
		sym := rec.QuoteKey
		if sym == "" {
			sym = TMXSymbol(rec.Symbol)
		}
		source, key, ok := QuoteSource(rec)
		if sym == "" || !ok || seen[sym] {
			continue
		}
		seen[sym] = true
		age, known := ageOf(fetched[sym], now)
		if !known || age > time.Duration(maxAgeMinutes*float64(time.Minute)) {
			out = append(out, Needed{sym, source, key})
		}
	}
	return out
}

func (c *Client) FetchFor(source, key string, now time.Time, chains map[string]OptionChain) *store.Quote {
	switch source {
	case "tmx":
		return c.FetchTMXQuote(key)
	case "cboe_ca":
		return c.FetchCboeCaQuote(key)
	case "coinbase":
		return c.FetchCoinbaseSpot(key, now)
	case "yahoo_quote":
		return c.FetchYahooQuote(key)
	case "cboe_options":
		if chains == nil {
			chains = map[string]OptionChain{}
		}
		root := OccRoot(key)
		chain, ok := chains[root]
		if !ok {
			chain = c.FetchCboeOptionChain(root)
			chains[root] = chain
		}
		return OptionMark(chain[key])
	}
	return nil
}

type PeekQuote struct {
	Price         *float64 `json:"price"`
	PriceChange   *float64 `json:"priceChange"`
	PercentChange *float64 `json:"percentChange"`
}

func (c *Client) PeekQuote(rec Rec, now time.Time) *PeekQuote {
	source, key, ok := QuoteSource(rec)
	if !ok {
		return nil
	}
	pk := strings.ToUpper(strings.TrimSpace(rec.Symbol)) + "@" + strings.ToUpper(strings.TrimSpace(rec.Exchange))
	c.peekMu.Lock()
	hit, has := c.peek[pk]
	c.peekMu.Unlock()
	if has && time.Since(hit.at) < PeekSeconds*time.Second {
		q := hit.quote
		return &q
	}
	q := c.FetchFor(source, key, now, nil)
	if q == nil || q.Price == nil {
		return nil
	}
	out := PeekQuote{Price: q.Price, PriceChange: q.PriceChange, PercentChange: q.PercentChange}
	c.peekMu.Lock()
	c.peek[pk] = peekHit{time.Now(), out}
	c.peekMu.Unlock()
	return &out
}

func (c *Client) ClearPeek() {
	c.peekMu.Lock()
	c.peek = map[string]peekHit{}
	c.peekMu.Unlock()
}

func (c *Client) RefreshQuotes(symbols []Rec, now time.Time) int {
	done := 0
	chains := map[string]OptionChain{}
	for _, n := range c.QuoteSymbolsNeedingRefresh(symbols, now, QuoteRefreshMinutes) {
		rec := c.FetchFor(n.Source, n.Key, now, chains)
		if rec != nil && rec.Price != nil {
			q := *rec
			q.Source = n.Source
			c.Store.UpsertQuote(n.Symbol, q, n.Source)
			done++
		}
	}
	return done
}

func (c *Client) StaleSymbols(symbols []Rec, now time.Time) []string {
	fetched := c.Store.DistributionsFetchedAt()
	out := []string{}
	for _, rec := range symbols {
		sym := TMXSymbol(rec.Symbol)
		if sym == "" || !IsCanadianListing(rec.Exchange, rec.Currency) {
			continue
		}
		age, known := ageOf(fetched[sym], now)
		if !known || age > RecordStaleHours*time.Hour {
			out = append(out, sym)
		}
	}
	return out
}

func (c *Client) RefreshDistributions(symbols []Rec, force bool, now time.Time) int {
	var todo []string
	if force {
		for _, r := range symbols {
			if IsCanadianListing(r.Exchange, r.Currency) {
				todo = append(todo, TMXSymbol(r.Symbol))
			}
		}
	} else {
		todo = c.StaleSymbols(symbols, now)
	}
	exchanges := map[string]string{}
	for _, r := range symbols {
		exchanges[TMXSymbol(r.Symbol)] = r.Exchange
	}
	done := 0
	for _, sym := range todo {
		exchange := exchanges[sym]
		quote, divs := c.FetchTMX(sym, exchange)
		if quote != nil && !in(CboeCanadaExchanges, strings.ToUpper(strings.TrimSpace(exchange))) {
			c.Store.UpsertQuote(sym, *quote, "tmx")
		}
		if len(divs) > 0 {
			c.Store.UpsertDistributions(sym, divs, "tmx")
		}
		if quote != nil || len(divs) > 0 {
			c.Store.MarkDistributionsFetched(sym, stamp(now))
			done++
		}
	}
	return done
}

func (c *Client) BenchmarkStale(today time.Time) bool {
	limit := py.DateStr(today.AddDate(0, 0, -StaleDays))
	for _, sym := range store.BenchmarkSymbols {
		last := c.Store.BenchmarkLastDate(sym)
		if last == "" || last < limit {
			return true
		}
	}
	return false
}

func (c *Client) IsStale(today time.Time, symbols []Rec) bool {
	limit := py.DateStr(today.AddDate(0, 0, -StaleDays))
	fx := c.Store.FXLastDate()
	if fx == "" || fx < limit || c.BenchmarkStale(today) {
		return true
	}
	return len(c.StaleSymbols(symbols, c.now())) > 0
}

type RefreshResult struct {
	FX            int  `json:"fx"`
	Benchmark     int  `json:"benchmark"`
	Distributions int  `json:"distributions"`
	Skipped       bool `json:"skipped"`
}

func (c *Client) beginRefresh() bool {
	c.refreshMu.Lock()
	defer c.refreshMu.Unlock()
	if c.refreshing {
		return false
	}
	c.refreshing = true
	return true
}

func (c *Client) endRefresh() {
	c.refreshMu.Lock()
	c.refreshing = false
	c.refreshMu.Unlock()
}

func (c *Client) RefreshAll(symbols []Rec) RefreshResult {
	if !c.beginRefresh() {
		return RefreshResult{Skipped: true}
	}
	defer c.endRefresh()
	c.Store.SetMeta("market_attempt_at", stamp(c.now()))
	return RefreshResult{FX: c.RefreshFX(), Benchmark: c.RefreshBenchmark() + c.RefreshTSX(), Distributions: c.RefreshDistributions(symbols, false, c.now())}
}

var torontoTZ = loadZone("America/Toronto")

func loadZone(name string) *time.Location {
	loc, err := time.LoadLocation(name)
	if err != nil {
		return time.UTC
	}
	return loc
}

func (c *Client) FXDayPublishedButMissing(now time.Time) bool {
	et := now.In(torontoTZ)
	if et.Weekday() == time.Saturday || et.Weekday() == time.Sunday {
		return false
	}
	if et.Hour() < 16 || (et.Hour() == 16 && et.Minute() < 30) {
		return false
	}
	return c.Store.FXLastDate() < py.DateStr(et)
}

func (c *Client) RefreshPeriodic(symbols []Rec, now time.Time) RefreshResult {
	if !c.beginRefresh() {
		return RefreshResult{Skipped: true}
	}
	defer c.endRefresh()
	out := RefreshResult{}
	last := c.Store.GetMeta("market_attempt_at")
	age, known := ageOf(last, now)
	if !known || age > MarketAttemptHours*time.Hour || c.FXDayPublishedButMissing(now) || c.BenchmarkStale(now) {
		c.Store.SetMeta("market_attempt_at", stamp(now))
		out.FX = c.RefreshFX()
		out.Benchmark = c.RefreshBenchmark() + c.RefreshTSX()
	}
	out.Distributions = c.RefreshDistributions(symbols, false, now)
	return out
}

func (c *Client) RefreshInBackground(symbols []Rec) {
	go c.RefreshAll(symbols)
}

var _ = sort.Strings
