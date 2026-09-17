package shorts

import (
	"encoding/csv"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"os"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/browserhttp"
	"github.com/ProfessorBagholder/Bagholder/internal/exposure"
	"github.com/ProfessorBagholder/Bagholder/internal/instruments"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
	"github.com/ProfessorBagholder/Bagholder/internal/xls"
)

const (
	USPositionURL = "https://api.finra.org/data/group/otcMarket/name/consolidatedShortInterest"
	USVolumeURL   = "https://cdn.finra.org/equity/regsho/daily/CNMSshvol%s.txt"
	CAPositionURL = "https://www.ciro.ca/sites/default/files/epubs/CSPR/%s_CSPR_Report.xls"
	CAVolumeURL   = "https://www.ciro.ca/sites/default/files/epubs/SSALE/%s-%s_ShortSaleTradingSummaryReport.csv"
	CACboeURL     = "https://www-api.cboe.com/ca/equities/listing-directory-data/"
	FileHours     = 6
	Tries         = 6
	Series        = 8
	FloatHours    = 12
	FloatMissMin  = 20
	YahooQuoteURL = "https://finance.yahoo.com/quote/%s/"
	YahooCrumbURL = "https://query1.finance.yahoo.com/v1/test/getcrumb"
	YahooStatsURL = "https://query1.finance.yahoo.com/v10/finance/quoteSummary/%s?modules=defaultKeyStatistics&crumb=%s"
	TMXUnitsQuery = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol shareOutStanding } }"
)

var CboeFunds = []string{"etf", "cef"}
var CAVenues = map[string][]string{"TSX": {"TSX"}, "TSXV": {"TSX-V", "TSXV"}, "CSE": {"CSE"}, "AQL": {"CBOE CANADA", "NEO"}}
var CAVenueNames = map[string]string{"TSX": "TSX", "TSXV": "TSX-V", "CSE": "CSE", "AQL": "Cboe Canada"}
var Headers = map[string]string{"User-Agent": market.UA, "Accept": "*/*"}

type fileTable struct {
	key  string
	rows map[string]map[string]any
	at   time.Time
}

type floatHit struct {
	value *float64
	at    time.Time
}

type Client struct {
	Market  *market.Client
	Store   *store.Store
	mu      sync.Mutex
	files   map[string]*fileTable
	shares  map[string]floatHit
	yahoo   *browserhttp.Session
	crumb   string
	NoYahoo bool
}

func NewClient(m *market.Client) *Client {
	return &Client{Market: m, Store: m.Store, files: map[string]*fileTable{}, shares: map[string]floatHit{}}
}

func num(v any) *float64 {
	s := strings.TrimSpace(strings.ReplaceAll(py.S(v), ",", ""))
	f, ok := py.NumOK(s)
	if !ok {
		return nil
	}
	return &f
}

func MarketOf(symbol, exchange, currency string) string {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	ex := strings.ToUpper(strings.TrimSpace(exchange))
	if sym == "" || symbols.IsOption(sym) || ex == "CRYPTO" || instruments.Find(sym, ex) != nil {
		return ""
	}
	form := market.TMXForm(ex, currency)
	if form == nil {
		return ""
	}
	if *form == ":US" {
		return "us"
	}
	return "ca"
}

func lastOfMonth(year int, month time.Month) time.Time {
	return time.Date(year, month+1, 1, 0, 0, 0, 0, time.UTC).AddDate(0, 0, -1)
}

func PositionDates(today time.Time, back int) []time.Time {
	var out []time.Time
	year, month := today.Year(), today.Month()
	for len(out) < back {
		last := lastOfMonth(year, month)
		for _, d := range []time.Time{last, time.Date(year, month, 15, 0, 0, 0, 0, time.UTC)} {
			if !d.After(today) && len(out) < back {
				out = append(out, d)
			}
		}
		if month == time.January {
			year, month = year-1, time.December
		} else {
			month--
		}
	}
	return out
}

type period struct{ start, end time.Time }

func VolumePeriods(today time.Time, back int) []period {
	var out []period
	year, month := today.Year(), today.Month()
	for len(out) < back {
		last := lastOfMonth(year, month)
		for _, p := range []period{{time.Date(year, month, 16, 0, 0, 0, 0, time.UTC), last}, {time.Date(year, month, 1, 0, 0, 0, 0, time.UTC), time.Date(year, month, 15, 0, 0, 0, 0, time.UTC)}} {
			if !p.end.After(today) && len(out) < back {
				out = append(out, p)
			}
		}
		if month == time.January {
			year, month = year-1, time.December
		} else {
			month--
		}
	}
	return out
}

func TradingDays(today time.Time, back int) []time.Time {
	var out []time.Time
	d := today
	for len(out) < back {
		if d.Weekday() != time.Saturday && d.Weekday() != time.Sunday {
			out = append(out, d)
		}
		d = d.AddDate(0, 0, -1)
	}
	return out
}

func dateOnly(t time.Time) time.Time {
	return time.Date(t.Year(), t.Month(), t.Day(), 0, 0, 0, 0, time.UTC)
}

func (c *Client) table(name string, build func(now time.Time) (string, map[string]map[string]any, bool), now time.Time) *fileTable {
	c.mu.Lock()
	held := c.files[name]
	if held != nil && time.Since(held.at) < FileHours*time.Hour {
		c.mu.Unlock()
		return held
	}
	c.mu.Unlock()
	key, rows, ok := build(now)
	c.mu.Lock()
	defer c.mu.Unlock()
	if !ok {
		held = c.files[name]
		if held == nil {
			held = &fileTable{rows: map[string]map[string]any{}}
		}
		held.at = time.Now()
	} else {
		held = &fileTable{key: key, rows: rows, at: time.Now()}
	}
	c.files[name] = held
	return held
}

func ParseUSVolume(text string) map[string]map[string]any {
	rows := map[string]map[string]any{}
	lines := py.Lines(text)
	if len(lines) > 0 {
		lines = lines[1:]
	}
	for _, line := range lines {
		parts := strings.Split(strings.TrimSpace(line), "|")
		if len(parts) < 5 {
			continue
		}
		sym := strings.ToUpper(strings.TrimSpace(parts[1]))
		short, total := num(parts[2]), num(parts[4])
		if sym != "" && short != nil && total != nil && *total != 0 {
			rows[sym] = map[string]any{"shortVolume": *short, "totalVolume": *total}
		}
	}
	return rows
}

func (c *Client) usVolumeFile(now time.Time) (string, map[string]map[string]any, bool) {
	for _, d := range TradingDays(dateOnly(now), Tries) {
		text, err := c.Market.GetText(fmt.Sprintf(USVolumeURL, d.Format("20060102")), Headers)
		if err != nil {
			continue
		}
		rows := ParseUSVolume(text)
		if len(rows) > 0 {
			return d.Format("2006-01-02"), rows, true
		}
	}
	return "", nil, false
}

func ParseCAPositions(grid [][]any) map[string]map[string]any {
	rows := map[string]map[string]any{}
	for _, r := range grid {
		if len(r) < 5 {
			continue
		}
		sym := strings.ToUpper(strings.TrimSpace(py.S(r[1])))
		shares := num(r[3])
		if sym == "" || shares == nil {
			continue
		}
		row := map[string]any{"venue": strings.ToUpper(strings.TrimSpace(py.S(r[2]))), "shares": *shares, "name": strings.TrimSpace(py.S(r[0]))}
		if ch := num(r[4]); ch != nil {
			row["change"] = *ch
		} else {
			row["change"] = nil
		}
		rows[sym] = row
	}
	return rows
}

func (c *Client) fetchRaw(url string) ([]byte, error) {
	return c.Market.FetchRaw(url, Headers)
}

func (c *Client) caPositionFile(now time.Time) (string, map[string]map[string]any, bool) {
	for _, d := range PositionDates(dateOnly(now), Tries) {
		raw, err := c.fetchRaw(fmt.Sprintf(CAPositionURL, d.Format("20060102")))
		var rows map[string]map[string]any
		if err == nil {
			grid, gerr := xls.Table(raw)
			if gerr == nil {
				rows = ParseCAPositions(grid)
			} else {
				err = gerr
			}
		}
		if err != nil {
			c.Market.NoteSource("ciro", false, fmt.Errorf("no report for %s", d.Format("2006-01-02")))
			continue
		}
		if len(rows) > 0 {
			c.Market.NoteSource("ciro", true, nil)
			return d.Format("2006-01-02"), rows, true
		}
	}
	return "", nil, false
}

func (c *Client) CATraded(symbol, exchange, currency, span string) *float64 {
	start, end, _ := strings.Cut(span, "/")
	if end == "" {
		return nil
	}
	code := market.TMXQuoteSymbol(symbol, exchange, currency)
	if code == "" {
		return nil
	}
	ask := func(form string) ([]market.Daily, error) {
		data, err := c.Market.PostJSON(market.TMXURL, map[string]any{"operationName": "getTimeSeriesData", "variables": map[string]any{"symbol": form, "freq": "day", "interval": 1, "start": start, "end": end}, "query": market.TMXHistoryQuery}, market.TMXHeaders)
		if err != nil {
			return nil, err
		}
		return market.ParseTMXHistory(data), nil
	}
	bars, err := c.Market.TMXLookupDaily(code, ask)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder shorts: %s traded volume failed: %s\n", symbol, err)
		return nil
	}
	total := 0.0
	any := false
	for _, b := range bars {
		if b.Volume != nil && *b.Volume != 0 {
			total += *b.Volume
			any = true
		}
	}
	if !any {
		return nil
	}
	return &total
}

func ParseCAVolume(text string) map[string]map[string]any {
	rows := map[string]map[string]any{}
	r := csv.NewReader(strings.NewReader(strings.TrimPrefix(text, "\ufeff")))
	r.FieldsPerRecord = -1
	r.LazyQuotes = true
	header, err := r.Read()
	if err != nil {
		return rows
	}
	idx := map[string]int{}
	for i, h := range header {
		idx[h] = i
	}
	get := func(rec []string, name string) string {
		if i, ok := idx[name]; ok && i < len(rec) {
			return rec[i]
		}
		return ""
	}
	for {
		rec, err := r.Read()
		if err == io.EOF {
			break
		}
		if err != nil {
			continue
		}
		sym := strings.ToUpper(strings.TrimSpace(get(rec, "Security")))
		short, pct := num(get(rec, "Short Traded Volume")), num(get(rec, "% Total Traded Volume"))
		if sym == "" || short == nil {
			continue
		}
		row := map[string]any{"venue": strings.ToUpper(strings.TrimSpace(get(rec, "Listing Market"))), "shortVolume": *short, "volumePct": nil, "totalVolume": nil}
		if pct != nil {
			row["volumePct"] = *pct
			if *pct != 0 {
				row["totalVolume"] = *short / *pct * 100
			}
		}
		rows[sym] = row
	}
	return rows
}

func (c *Client) caVolumeFile(now time.Time) (string, map[string]map[string]any, bool) {
	for _, p := range VolumePeriods(dateOnly(now), Tries) {
		text, err := c.Market.GetText(fmt.Sprintf(CAVolumeURL, p.start.Format("20060102"), p.end.Format("20060102")), Headers)
		if err != nil {
			continue
		}
		rows := ParseCAVolume(text)
		if len(rows) > 0 {
			return p.start.Format("2006-01-02") + "/" + p.end.Format("2006-01-02"), rows, true
		}
	}
	return "", nil, false
}

func (c *Client) caPositionsOn(day string) map[string]map[string]any {
	name := "ca_position:" + day
	c.mu.Lock()
	held := c.files[name]
	c.mu.Unlock()
	if held != nil {
		return held.rows
	}
	rows := map[string]map[string]any{}
	if raw, err := c.fetchRaw(fmt.Sprintf(CAPositionURL, strings.ReplaceAll(day, "-", ""))); err == nil {
		if grid, err := xls.Table(raw); err == nil {
			rows = ParseCAPositions(grid)
		}
	}
	c.mu.Lock()
	c.files[name] = &fileTable{key: day, rows: rows, at: time.Now()}
	c.mu.Unlock()
	return rows
}

func (c *Client) CASeries(symbol, exchange, asof string, now time.Time, back int) []store.ShortPoint {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	out := []store.ShortPoint{}
	for _, d := range PositionDates(dateOnly(now), back) {
		day := d.Format("2006-01-02")
		if asof != "" && day > asof {
			continue
		}
		row := c.caPositionsOn(day)[sym]
		if row != nil && venueFits(py.S(row["venue"]), exchange) {
			shares := row["shares"].(float64)
			out = append(out, store.ShortPoint{Date: day, Shares: &shares})
		}
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Date < out[j].Date })
	return out
}

func (c *Client) yahooSession() (*browserhttp.Session, string) {
	if c.NoYahoo {
		return nil, ""
	}
	c.mu.Lock()
	if c.yahoo != nil {
		s, crumb := c.yahoo, c.crumb
		c.mu.Unlock()
		return s, crumb
	}
	c.mu.Unlock()
	session, err := browserhttp.New(market.TimeoutSec, true)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder shorts: yahoo would not open: %s\n", err)
		return nil, ""
	}
	if _, err := session.Get(fmt.Sprintf(YahooQuoteURL, "AAPL"), nil); err != nil {
		fmt.Fprintf(os.Stderr, "bagholder shorts: yahoo would not open: %s\n", err)
		return nil, ""
	}
	resp, err := session.Get(YahooCrumbURL, nil)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder shorts: yahoo would not open: %s\n", err)
		return nil, ""
	}
	crumb := strings.TrimSpace(string(resp.Body))
	if crumb == "" || len(crumb) > 32 {
		return nil, ""
	}
	c.mu.Lock()
	c.yahoo, c.crumb = session, crumb
	c.mu.Unlock()
	return session, crumb
}

func (c *Client) cboeUnits(symbol string, now time.Time) *float64 {
	build := func(when time.Time) (string, map[string]map[string]any, bool) {
		text, err := c.Market.GetText(CACboeURL, Headers)
		if err != nil {
			return "", nil, false
		}
		var d struct {
			Data []map[string]any `json:"data"`
		}
		if err := json.Unmarshal([]byte(text), &d); err != nil {
			return "", nil, false
		}
		rows := map[string]map[string]any{}
		for _, r := range d.Data {
			kind := strings.ToLower(strings.TrimSpace(py.S(r["security"])))
			if kind != "etf" && kind != "cef" {
				continue
			}
			cap, last := num(r["marketcap"]), num(r["last"])
			if cap == nil || last == nil || *cap == 0 || *last == 0 {
				continue
			}
			count := *cap / *last
			if math.Abs(count-math.RoundToEven(count)) < 1e-6 {
				rows[strings.ToUpper(strings.TrimSpace(py.S(r["symbol"])))] = map[string]any{"count": math.RoundToEven(count)}
			}
		}
		return "cboe", rows, true
	}
	held := c.table("cboe_listings", build, now)
	if r, ok := held.rows[strings.ToUpper(strings.TrimSpace(symbol))]; ok {
		v := r["count"].(float64)
		return &v
	}
	return nil
}

func (c *Client) fundUnits(symbol, exchange, currency string) *float64 {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if MarketOf(sym, exchange, currency) == "us" {
		return nil
	}
	var count *float64
	if code := market.TMXQuoteSymbol(sym, exchange, currency); code != "" {
		ask := func(form string) (map[string]any, error) {
			answered, err := c.Market.PostJSON(market.TMXURL, map[string]any{"operationName": "getQuoteBySymbol", "variables": map[string]any{"symbol": form, "locale": "en"}, "query": TMXUnitsQuery}, market.TMXHeaders)
			if err != nil {
				return nil, err
			}
			data, _ := answered["data"].(map[string]any)
			q, _ := data["getQuoteBySymbol"].(map[string]any)
			return q, nil
		}
		first := c.Market.TMXRemembered(code)
		q, err := ask(first)
		if err == nil && len(q) == 0 && !strings.HasPrefix(code, "^") {
			if alt := c.Market.TMXResolve(code); alt != "" && alt != first {
				q, err = ask(alt)
			}
		}
		if err != nil {
			fmt.Fprintf(os.Stderr, "bagholder shorts: %s units failed: %s\n", sym, err)
		} else if v := num(q["shareOutStanding"]); v != nil && *v != 0 {
			count = v
		}
	}
	if count != nil {
		return count
	}
	if !in(CAVenues["AQL"], strings.ToUpper(strings.TrimSpace(exchange))) {
		return nil
	}
	return c.cboeUnits(sym, c.Market.Clock())
}

func in(list []string, s string) bool {
	for _, x := range list {
		if x == s {
			return true
		}
	}
	return false
}

func (c *Client) FloatShares(symbol, exchange, currency, name string) *float64 {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	key := sym + "|" + strings.ToUpper(strings.TrimSpace(exchange))
	c.mu.Lock()
	if held, ok := c.shares[key]; ok {
		ttl := FloatMissMin * time.Minute
		if held.value != nil && *held.value != 0 {
			ttl = FloatHours * time.Hour
		}
		if time.Since(held.at) < ttl {
			c.mu.Unlock()
			return held.value
		}
	}
	c.mu.Unlock()
	var count *float64
	fund := exposure.IsFund(name)
	where := MarketOf(sym, exchange, currency)
	ccy := strings.TrimSpace(currency)
	if ccy == "" {
		ccy = "CAD"
		if where == "us" {
			ccy = "USD"
		}
	}
	session, crumb := c.yahooSession()
	if session != nil {
		forms := market.YahooFormsFor(market.Rec{Symbol: sym, Exchange: exchange, Currency: ccy})
		if len(forms) == 0 {
			forms = []string{market.TMXSymbol(sym)}
		}
		for _, form := range forms {
			resp, ok := c.Market.YahooPaced(func() (*browserhttp.Response, error) {
				return session.Get(fmt.Sprintf(YahooStatsURL, form, crumb), nil)
			})
			if !ok || resp == nil {
				continue
			}
			if resp.Status != 200 {
				continue
			}
			var d map[string]any
			if err := json.Unmarshal(resp.Body, &d); err != nil {
				fmt.Fprintf(os.Stderr, "bagholder shorts: %s float from yahoo failed: %s\n", form, err)
				continue
			}
			qs, _ := d["quoteSummary"].(map[string]any)
			results, _ := qs["result"].([]any)
			stats := map[string]any{}
			if len(results) > 0 {
				res, _ := results[0].(map[string]any)
				stats, _ = res["defaultKeyStatistics"].(map[string]any)
			}
			pick := func(field string) *float64 {
				v := stats[field]
				if m, ok := v.(map[string]any); ok {
					return num(m["raw"])
				}
				return num(v)
			}
			count = pick("floatShares")
			if (count == nil || *count == 0) && fund {
				count = pick("sharesOutstanding")
			}
			if count != nil && *count != 0 {
				break
			}
		}
	}
	if (count == nil || *count == 0) && fund {
		count = c.fundUnits(sym, exchange, ccy)
	}
	if count != nil && *count == 0 {
		count = nil
	}
	c.mu.Lock()
	c.shares[key] = floatHit{count, time.Now()}
	c.mu.Unlock()
	return count
}

func venueFits(code, exchange string) bool {
	ex := strings.ToUpper(strings.TrimSpace(exchange))
	return ex == "" || in(CAVenues[strings.ToUpper(strings.TrimSpace(code))], ex)
}

type Record = store.Short

func (c *Client) USPosition(symbol string, now time.Time) (Record, bool) {
	body := map[string]any{"limit": 20,
		"compareFilters":   []map[string]any{{"fieldName": "symbolCode", "fieldValue": strings.ToUpper(strings.TrimSpace(symbol)), "compareType": "EQUAL"}},
		"dateRangeFilters": []map[string]any{{"fieldName": "settlementDate", "startDate": now.AddDate(0, 0, -150).Format("2006-01-02"), "endDate": now.Format("2006-01-02")}}}
	answered, err := c.Market.PostJSONList(USPositionURL, body, map[string]string{"Accept": "application/json"})
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder shorts: %s position from finra failed: %s\n", symbol, err)
		return Record{}, false
	}
	var rows []map[string]any
	for _, r := range answered {
		if py.S(r["settlementDate"]) != "" {
			rows = append(rows, r)
		}
	}
	if len(rows) == 0 {
		return Record{}, false
	}
	sort.SliceStable(rows, func(i, j int) bool { return py.S(rows[i]["settlementDate"]) > py.S(rows[j]["settlementDate"]) })
	r := rows[0]
	rec := Record{AsOf: cut(py.S(r["settlementDate"]), 10), Shares: num(r["currentShortPositionQuantity"]), Previous: num(r["previousShortPositionQuantity"]), Change: num(r["changePreviousNumber"]), AverageVolume: num(r["averageDailyVolumeQuantity"]), Series: []store.ShortPoint{}}
	if len(rows) > 1 {
		rec.PreviousOf = cut(py.S(rows[1]["settlementDate"]), 10)
	}
	asc := append([]map[string]any{}, rows...)
	sort.SliceStable(asc, func(i, j int) bool { return py.S(asc[i]["settlementDate"]) < py.S(asc[j]["settlementDate"]) })
	for _, x := range asc {
		if v := num(x["currentShortPositionQuantity"]); v != nil {
			rec.Series = append(rec.Series, store.ShortPoint{Date: cut(py.S(x["settlementDate"]), 10), Shares: v})
		}
	}
	return rec, true
}

func cut(s string, n int) string {
	if len(s) > n {
		return s[:n]
	}
	return s
}

func (c *Client) USVolume(symbol string, now time.Time, rec *Record) {
	held := c.table("us_volume", c.usVolumeFile, now)
	row := held.rows[strings.ToUpper(strings.TrimSpace(symbol))]
	if row == nil {
		return
	}
	short := row["shortVolume"].(float64)
	total := row["totalVolume"].(float64)
	rec.VolumeOf, rec.VolumeSpan = held.key, "day"
	rec.ShortVolume, rec.TotalVolume = &short, &total
	if total != 0 {
		rec.VolumePct = py.Ptr(short / total * 100)
	}
}

func (c *Client) CAPosition(symbol, exchange string, now time.Time) (Record, string, string, bool) {
	held := c.table("ca_position", c.caPositionFile, now)
	row := held.rows[strings.ToUpper(strings.TrimSpace(symbol))]
	if row == nil || !venueFits(py.S(row["venue"]), exchange) {
		return Record{}, "", "", false
	}
	shares := row["shares"].(float64)
	rec := Record{AsOf: held.key, Shares: &shares}
	if ch, ok := row["change"].(float64); ok {
		rec.Change = &ch
		rec.Previous = py.Ptr(shares - ch)
	}
	for _, d := range PositionDates(dateOnly(now), Tries) {
		if day := d.Format("2006-01-02"); day < held.key {
			rec.PreviousOf = day
			break
		}
	}
	return rec, strings.ToUpper(strings.TrimSpace(py.S(row["venue"]))), strings.TrimSpace(py.S(row["name"])), true
}

func (c *Client) CAVolume(symbol, exchange, currency string, now time.Time, rec *Record) {
	held := c.table("ca_volume", c.caVolumeFile, now)
	if held.key == "" {
		return
	}
	row := held.rows[strings.ToUpper(strings.TrimSpace(symbol))]
	if row != nil && !venueFits(py.S(row["venue"]), exchange) {
		return
	}
	if row == nil {
		traded := c.CATraded(symbol, exchange, currency, held.key)
		if traded == nil || *traded == 0 {
			return
		}
		rec.VolumeOf, rec.VolumeSpan = held.key, "period"
		rec.ShortVolume, rec.TotalVolume, rec.VolumePct = py.Ptr(0), traded, py.Ptr(0)
		return
	}
	rec.VolumeOf, rec.VolumeSpan = held.key, "period"
	short := row["shortVolume"].(float64)
	rec.ShortVolume = &short
	if v, ok := row["totalVolume"].(float64); ok {
		rec.TotalVolume = &v
	}
	if v, ok := row["volumePct"].(float64); ok {
		rec.VolumePct = &v
	}
}

func (c *Client) AverageVolume(rec *Record) *float64 {
	if rec.Market == "us" {
		return rec.AverageVolume
	}
	if rec.TotalVolume == nil || *rec.TotalVolume == 0 || !strings.Contains(rec.VolumeOf, "/") {
		return nil
	}
	start, end, _ := strings.Cut(rec.VolumeOf, "/")
	days := c.Store.BenchmarkDays("TSX", start, end)
	if days == 0 {
		return nil
	}
	return py.Ptr(*rec.TotalVolume / float64(days))
}

func (c *Client) DaysToCover(rec *Record) *float64 {
	average := c.AverageVolume(rec)
	if rec.Shares == nil || *rec.Shares == 0 || average == nil || *average == 0 {
		return nil
	}
	return py.Ptr(py.Round(*rec.Shares / *average, 1))
}

func (c *Client) ForListing(symbol, exchange, currency string, now time.Time, trend bool, name string) (Record, bool) {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	where := MarketOf(sym, exchange, currency)
	if where == "" {
		return Record{}, false
	}
	var rec Record
	venue, issuer := "", ""
	if where == "us" {
		rec, _ = c.USPosition(sym, now)
		c.USVolume(sym, now, &rec)
	} else {
		rec, venue, issuer, _ = c.CAPosition(sym, exchange, now)
		c.CAVolume(sym, exchange, currency, now, &rec)
	}
	if where == "ca" && trend {
		rec.Series = c.CASeries(sym, exchange, rec.AsOf, now, Series)
	}
	rec.Symbol = sym
	rec.Exchange = strings.ToUpper(strings.TrimSpace(exchange))
	if rec.Exchange == "" {
		rec.Exchange = CAVenueNames[venue]
	}
	rec.Market = where
	rec.Source = "FINRA"
	if where != "us" {
		rec.Source = "CIRO"
	}
	if issuer != "" {
		rec.Name = issuer
	}
	floatName := strings.TrimSpace(name)
	if floatName == "" {
		floatName = issuer
	}
	floated := c.FloatShares(sym, exchange, currency, floatName)
	rec.Float = floated
	rec.OfFloat = nil
	if floated != nil && *floated != 0 && rec.Shares != nil && *rec.Shares != 0 {
		rec.OfFloat = py.Ptr(*rec.Shares / *floated * 100)
	}
	rec.AverageVolume = c.AverageVolume(&rec)
	rec.DaysToCover = c.DaysToCover(&rec)
	return rec, true
}
