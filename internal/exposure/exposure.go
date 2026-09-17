package exposure

import (
	"encoding/csv"
	"encoding/json"
	"io"
	"net/http"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"

	"golang.org/x/net/html"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const (
	UA        = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
	PaceSec   = 0.6
	MaxDepth  = 3
	FreshDays = 7
)

var Headers = map[string]string{"User-Agent": UA, "Accept": "text/html,application/json;q=0.9,*/*;q=0.8", "Accept-Language": "en-CA,en;q=0.9"}

func num(v any) *float64 {
	s := strings.TrimSpace(py.S(v))
	s = strings.ReplaceAll(strings.ReplaceAll(s, ",", ""), "%", "")
	if strings.HasPrefix(s, "(") && strings.HasSuffix(s, ")") {
		s = "-" + s[1:len(s)-1]
	}
	f, ok := py.NumOK(s)
	if !ok {
		return nil
	}
	return &f
}

func numOr(v any, def float64) float64 {
	if p := num(v); p != nil {
		return *p
	}
	return def
}

type Holding struct {
	Ticker   string
	Exchange string
	Currency string
	Name     string
	Weight   float64
	Sector   string
	Country  string
	Fund     bool
}

type Breakdown struct {
	Sectors   map[string]float64
	Countries map[string]float64
	Holdings  []Holding
	Source    string
	AsOf      string
}

type SearchMatch struct {
	Symbol   string
	Exchange string
	Currency string
}

type Client struct {
	Market       *market.Client
	Store        *store.Store
	SymbolSearch func(text string) []SearchMatch
	pacer        *market.Pacer
	mu           sync.Mutex
	vanguardMap  map[string]string
	isharesMap   map[string]string
	ninepoint    map[string]string
	yahooCookie  string
	yahooCrumb   string
}

func NewClient(m *market.Client) *Client {
	return &Client{Market: m, Store: m.Store, pacer: market.NewPacer(), vanguardMap: map[string]string{}, isharesMap: map[string]string{}, ninepoint: map[string]string{}}
}

func hostOf(url string) string {
	parts := strings.Split(url, "/")
	if len(parts) > 2 {
		return parts[2]
	}
	return url
}

func merged(extra map[string]string) map[string]string {
	out := map[string]string{}
	for k, v := range Headers {
		out[k] = v
	}
	for k, v := range extra {
		out[k] = v
	}
	return out
}

func (c *Client) get(url string, headers map[string]string) (string, error) {
	c.pacer.Pace(hostOf(url), PaceSec)
	return c.Market.GetText(url, merged(headers))
}

func (c *Client) post(url string, payload any, headers map[string]string) (map[string]any, error) {
	c.pacer.Pace(hostOf(url), PaceSec)
	return c.Market.PostJSON(url, payload, merged(headers))
}

const (
	TMXSectorQuery   = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name sector industry exchangeName } }"
	NasdaqSummaryURL = "https://api.nasdaq.com/api/quote/%s/summary?assetclass=stocks"
)

func (c *Client) tmxRecord(key string) map[string]any {
	if key == "" {
		return nil
	}
	c.pacer.Pace("app-money.tmx.com", PaceSec)
	d, err := c.Market.PostJSON(market.TMXURL, map[string]any{"operationName": "getQuoteBySymbol", "variables": map[string]any{"symbol": key, "locale": "en"}, "query": TMXSectorQuery}, market.TMXHeaders)
	if err != nil {
		return nil
	}
	data, _ := d["data"].(map[string]any)
	q, _ := data["getQuoteBySymbol"].(map[string]any)
	if py.S(q["sector"]) != "" || py.S(q["industry"]) != "" || py.S(q["name"]) != "" {
		return q
	}
	return nil
}

func (c *Client) nasdaqSummary(symbol string) (string, string) {
	raw, err := c.get(strings.Replace(NasdaqSummaryURL, "%s", symbol, 1), map[string]string{"Accept": "application/json, text/plain, */*"})
	if err != nil {
		return "", ""
	}
	var d map[string]any
	if json.Unmarshal([]byte(raw), &d) != nil {
		return "", ""
	}
	data, _ := d["data"].(map[string]any)
	s, _ := data["summaryData"].(map[string]any)
	sector, _ := s["Sector"].(map[string]any)
	industry, _ := s["Industry"].(map[string]any)
	return py.S(sector["value"]), py.S(industry["value"])
}

type ShareClass struct {
	Sector, Industry, Country, Source string
}

var cdrRE = regexp.MustCompile(`\bCDR\b`)

func (c *Client) ClassifyShare(symbol, exchange, currency string) ShareClass {
	sym := market.TMXSymbol(symbol)
	country := VenueCountryOf(exchange)
	out := ShareClass{Country: country}
	key := market.TMXQuoteSymbol(symbol, exchange, currency)
	if key == "" && market.TMXForm(exchange, currency) == nil && sym != "" && !strings.Contains(sym, " ") {
		key = sym
		if strings.ToUpper(currency) == "USD" {
			key = sym + ":US"
		}
	}
	if key != "" {
		first := c.Market.TMXRemembered(key)
		rec := c.tmxRecord(first)
		if rec == nil && !strings.HasPrefix(key, "^") {
			if alt := c.Market.TMXResolve(key); alt != "" && alt != first {
				rec = c.tmxRecord(alt)
			}
		}
		if rec != nil && country == "" && cdrRE.MatchString(py.S(rec["name"])) && !strings.HasSuffix(key, ":US") {
			if us := c.tmxRecord(market.TMXBare(key) + ":US"); us != nil {
				rec = us
			}
		}
		if rec != nil {
			out.Sector = NormSector(py.S(rec["sector"]))
			out.Industry = strings.TrimSpace(py.S(rec["industry"]))
			out.Country = country
			if out.Country == "" {
				out.Country = VenueCountryOf(py.S(rec["exchangeName"]))
			}
			out.Source = "TMX Money"
		}
	}
	if out.Sector == "" && (country == "United States" || strings.ToUpper(currency) == "USD") && sym != "" && !strings.Contains(sym, " ") {
		sector, industry := c.nasdaqSummary(sym)
		if sector != "" {
			out.Sector = NormSector(sector)
			if out.Industry == "" {
				out.Industry = industry
			}
			if out.Country == "" {
				out.Country = "United States"
			}
			if out.Source == "" {
				out.Source = "Nasdaq"
			}
		}
	}
	return out
}

func HTMLTables(doc string) [][][]string {
	tables := [][][]string{}
	var table [][]string
	var row []string
	var cell []string
	inTable, inRow, inCell := false, false, false
	z := html.NewTokenizer(strings.NewReader(doc))
	for {
		tt := z.Next()
		if tt == html.ErrorToken {
			break
		}
		switch tt {
		case html.StartTagToken, html.SelfClosingTagToken:
			name, _ := z.TagName()
			tag := strings.ToLower(string(name))
			switch tag {
			case "table":
				table, inTable = [][]string{}, true
			case "tr":
				if inTable {
					row, inRow = []string{}, true
				}
			case "td", "th":
				if inRow {
					cell, inCell = []string{}, true
				}
			case "br":
				if inCell {
					cell = append(cell, " ")
				}
			}
		case html.EndTagToken:
			name, _ := z.TagName()
			tag := strings.ToLower(string(name))
			switch tag {
			case "td", "th":
				if inCell && inRow {
					row = append(row, py.Strip(py.CollapseSpace(strings.Join(cell, ""))))
					inCell = false
				}
			case "tr":
				if inRow && inTable {
					if len(row) > 0 {
						table = append(table, row)
					}
					inRow = false
				}
			case "table":
				if inTable {
					tables = append(tables, table)
					inTable = false
				}
			}
		case html.TextToken:
			if inCell {
				cell = append(cell, string(z.Text()))
			}
		}
	}
	return tables
}

func headerIndex(header []string, names ...string) int {
	low := make([]string, len(header))
	for i, h := range header {
		low[i] = strings.ToLower(h)
	}
	for _, n := range names {
		for i, h := range low {
			if strings.Contains(h, n) {
				return i
			}
		}
	}
	return -1
}

const (
	VanguardGQL = "https://www.vanguard.ca/gpx/graphql"
)

var VanguardHeaders = map[string]string{"Content-Type": "application/json", "X-Consumer-ID": "ca0", "apollographql-client-name": "gpx", "Origin": "https://www.vanguard.ca", "Referer": "https://www.vanguard.ca/en/product"}
var VanguardPortIDs = []string{"1811", "1817", "1936", "9561", "9554", "9559", "9560", "9569", "9570", "9558", "9555", "9550", "9549", "9742", "9556", "9548", "9828", "9835", "9795", "9563", "9562",
	"9566", "9564", "9551", "9567", "9870", "9841", "9552", "9553", "9565", "9568", "9691", "9577", "9578", "9579", "9692", "9557", "9864", "9865", "9867", "9896"}

func (c *Client) vanguardPortID(symbol string) (string, error) {
	c.mu.Lock()
	empty := len(c.vanguardMap) == 0
	c.mu.Unlock()
	if empty {
		q := map[string]any{"operationName": "FundFinderFunds", "variables": map[string]any{"portIds": VanguardPortIDs},
			"query": "query FundFinderFunds($portIds: [String!]!) { funds(portIds: $portIds) { portId profile { fundFullName listings { identifiers(altIds: [\"Ticker - Canada\", \"Ticker\"]) { altId altIdValue } } } } }"}
		d, err := c.post(VanguardGQL, q, VanguardHeaders)
		if err != nil {
			return "", err
		}
		data, _ := d["data"].(map[string]any)
		funds, _ := data["funds"].([]any)
		c.mu.Lock()
		for _, raw := range funds {
			f, _ := raw.(map[string]any)
			p, _ := f["profile"].(map[string]any)
			listings, _ := p["listings"].([]any)
			for _, lraw := range listings {
				l, _ := lraw.(map[string]any)
				ids, _ := l["identifiers"].([]any)
				for _, iraw := range ids {
					i, _ := iraw.(map[string]any)
					if v := py.S(i["altIdValue"]); v != "" {
						pid := py.S(f["portId"])
						if pid == "" {
							pid = py.S(p["portId"])
						}
						if _, ok := c.vanguardMap[strings.ToUpper(v)]; !ok {
							c.vanguardMap[strings.ToUpper(v)] = pid
						}
					}
				}
			}
		}
		c.mu.Unlock()
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.vanguardMap[market.TMXSymbol(symbol)], nil
}

func firstFund(d map[string]any) map[string]any {
	data, _ := d["data"].(map[string]any)
	funds, _ := data["funds"].([]any)
	if len(funds) == 0 {
		return map[string]any{}
	}
	f, _ := funds[0].(map[string]any)
	return f
}

func (c *Client) VanguardCA(symbol, name, exchange string) (*Breakdown, error) {
	pid, err := c.vanguardPortID(symbol)
	if err != nil {
		return nil, err
	}
	if pid == "" {
		return nil, nil
	}
	sec, err := c.post(VanguardGQL, map[string]any{"operationName": "getSectorDiversification", "variables": map[string]any{"portIds": []string{pid}},
		"query": "query getSectorDiversification($portIds: [String!]!) { funds(portIds: $portIds) { sectorDiversification { sectorName fundPercent date } } }"}, VanguardHeaders)
	if err != nil {
		return nil, err
	}
	mkt, err := c.post(VanguardGQL, map[string]any{"operationName": "MarketAllocationGqlQuery", "variables": map[string]any{"portIds": []string{pid}},
		"query": "query MarketAllocationGqlQuery($portIds: [String!]!) { funds(portIds: $portIds) { marketAllocation { countryName fundMktPercent date } } }"}, VanguardHeaders)
	if err != nil {
		return nil, err
	}
	srows, _ := firstFund(sec)["sectorDiversification"].([]any)
	crows, _ := firstFund(mkt)["marketAllocation"].([]any)
	sectors, countries := map[string]float64{}, map[string]float64{}
	for _, raw := range srows {
		r, _ := raw.(map[string]any)
		n, w := NormSector(py.S(r["sectorName"])), numOr(r["fundPercent"], 0)
		if n != "" && w > 0 {
			sectors[n] += w
		}
	}
	for _, raw := range crows {
		r, _ := raw.(map[string]any)
		n, w := NormCountry(py.S(r["countryName"])), numOr(r["fundMktPercent"], 0)
		if n != "" && w > 0 {
			countries[n] += w
		}
	}
	if len(sectors) == 0 && len(countries) == 0 {
		return nil, nil
	}
	asOf := ""
	if len(srows) > 0 {
		r, _ := srows[0].(map[string]any)
		asOf = py.S(r["date"])
	} else if len(crows) > 0 {
		r, _ := crows[0].(map[string]any)
		asOf = py.S(r["date"])
	}
	return &Breakdown{Sectors: sectors, Countries: countries, Holdings: []Holding{}, Source: "Vanguard Canada", AsOf: asOf}, nil
}

const (
	ISharesScreener = "https://www.blackrock.com/ca/investors/en/product-screener/product-screener-v3.1.jsn?dcrPath=/templatedata/config/product-screener-v3/data/en/ca-one/product-screener-backend-config&siteEntryPassthrough=true"
	ISharesHoldings = "https://www.blackrock.com%s/1464253357814.ajax?fileType=csv&fileName=holdings&dataType=fund"
)

func (c *Client) isharesPage(symbol string) (string, error) {
	c.mu.Lock()
	empty := len(c.isharesMap) == 0
	c.mu.Unlock()
	if empty {
		raw, err := c.get(ISharesScreener, map[string]string{"Accept": "application/json, text/plain, */*"})
		if err != nil {
			return "", err
		}
		var d map[string]any
		if err := json.Unmarshal([]byte(strings.TrimPrefix(raw, "\ufeff")), &d); err != nil {
			return "", err
		}
		c.mu.Lock()
		for _, raw := range d {
			rec, ok := raw.(map[string]any)
			if ok && py.S(rec["localExchangeTicker"]) != "" && py.S(rec["productPageUrl"]) != "" {
				c.isharesMap[strings.ToUpper(py.S(rec["localExchangeTicker"]))] = py.S(rec["productPageUrl"])
			}
		}
		c.mu.Unlock()
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.isharesMap[market.TMXSymbol(symbol)], nil
}

func csvRow(line string) []string {
	r := csv.NewReader(strings.NewReader(line))
	r.FieldsPerRecord = -1
	r.LazyQuotes = true
	rec, err := r.Read()
	if err != nil {
		return []string{line}
	}
	return rec
}

func csvRows(lines []string) [][]string {
	r := csv.NewReader(strings.NewReader(strings.Join(lines, "\n")))
	r.FieldsPerRecord = -1
	r.LazyQuotes = true
	var out [][]string
	for {
		rec, err := r.Read()
		if err == io.EOF {
			break
		}
		if err != nil {
			if rec != nil {
				out = append(out, rec)
			}
			continue
		}
		out = append(out, rec)
	}
	return out
}

func at(r []string, i int) string {
	if i >= 0 && i < len(r) {
		return r[i]
	}
	return ""
}

func ParseISharesCSV(text string) ([]Holding, string) {
	lines := py.Lines(strings.TrimPrefix(text, "\ufeff"))
	asOf := ""
	start := -1
	for i, line := range lines {
		if strings.HasPrefix(line, "Fund Holdings as of") {
			parts := csvRow(line)
			if len(parts) > 1 {
				asOf = parts[1]
			}
		}
		if strings.HasPrefix(line, "Ticker,") || strings.HasPrefix(line, `"Ticker"`) {
			start = i
			break
		}
	}
	if start < 0 {
		return []Holding{}, asOf
	}
	rows := csvRows(lines[start:])
	if len(rows) == 0 {
		return []Holding{}, asOf
	}
	header := make([]string, len(rows[0]))
	for i, h := range rows[0] {
		header[i] = strings.TrimSpace(h)
	}
	it, iname, isec, icls, iw, iloc, iex, iccy := headerIndex(header, "ticker"), headerIndex(header, "name"), headerIndex(header, "sector"), headerIndex(header, "asset class"), headerIndex(header, "weight"), headerIndex(header, "location"), headerIndex(header, "exchange"), headerIndex(header, "currency")
	out := []Holding{}
	for _, r := range rows[1:] {
		mx := it
		if iw > mx {
			mx = iw
		}
		if len(r) <= mx || it < 0 || strings.TrimSpace(r[it]) == "" {
			continue
		}
		cls := strings.ToLower(strings.TrimSpace(at(r, icls)))
		w := 0.0
		if iw >= 0 {
			w = numOr(r[iw], 0)
		}
		if w <= 0 || cls == "cash" || cls == "money market" || cls == "futures" || cls == "derivatives" || cls == "forwards" || cls == "fx" {
			continue
		}
		nm := strings.TrimSpace(at(r, iname))
		h := Holding{Ticker: strings.TrimSpace(r[it]), Name: nm, Weight: w, Exchange: strings.TrimSpace(at(r, iex)), Currency: strings.TrimSpace(at(r, iccy)), Fund: strings.Contains(strings.ToUpper(nm), "ISHARES") || IsFund(nm)}
		if isec >= 0 {
			h.Sector = NormSector(at(r, isec))
		}
		if iloc >= 0 {
			h.Country = NormCountry(at(r, iloc))
		}
		out = append(out, h)
	}
	return out, asOf
}

func (c *Client) ISharesCA(symbol, name, exchange string) (*Breakdown, error) {
	page, err := c.isharesPage(symbol)
	if err != nil {
		return nil, err
	}
	if page == "" {
		return nil, nil
	}
	text, err := c.get(strings.Replace(ISharesHoldings, "%s", page, 1), map[string]string{"Accept": "text/csv,*/*"})
	if err != nil {
		return nil, err
	}
	holdings, asOf := ParseISharesCSV(text)
	if len(holdings) == 0 {
		return nil, nil
	}
	for i := range holdings {
		if holdings[i].Fund {
			holdings[i].Sector = ""
		}
	}
	return &Breakdown{Sectors: map[string]float64{}, Countries: map[string]float64{}, Holdings: holdings, Source: "iShares Canada", AsOf: asOf}, nil
}

const HarvestPage = "https://harvestportfolios.com/etf/%s/"

var cashRowRE = regexp.MustCompile(`(?i)written options|cash and other|cash & other`)
var holdingsHeadRE = regexp.MustCompile(`(?i)^holdings?\b`)
var refTickerRE = regexp.MustCompile(`^[A-Z0-9][A-Z0-9.:-]{0,9}$`)

func ParseHarvestTables(tables [][][]string) ([]Holding, string) {
	holdings := []Holding{}
	ref := ""
	for _, t := range tables {
		if len(t) == 0 {
			continue
		}
		header := t[0]
		for _, row := range t {
			if len(row) >= 2 && strings.HasPrefix(strings.ToLower(strings.TrimSpace(row[0])), "reference asset") {
				ref = strings.TrimSpace(row[1])
			}
		}
		it, iw := headerIndex(header, "ticker"), headerIndex(header, "weight")
		iname, isec, ictry := headerIndex(header, "name"), headerIndex(header, "sector"), headerIndex(header, "country")
		if it >= 0 && iw >= 0 && iname >= 0 {
			mx := it
			if iw > mx {
				mx = iw
			}
			if iname > mx {
				mx = iname
			}
			for _, r := range t[1:] {
				if len(r) <= mx {
					continue
				}
				w := numOr(r[iw], 0)
				nm, tk := strings.TrimSpace(r[iname]), strings.TrimSpace(r[it])
				if w <= 0 || nm == "" || cashRowRE.MatchString(nm) {
					continue
				}
				parts := py.Fields(tk)
				sym, code := tk, ""
				if len(parts) >= 2 {
					sym, code = parts[0], parts[1]
				}
				h := Holding{Ticker: sym, Name: nm, Weight: w, Fund: IsFund(nm)}
				if isec >= 0 && isec < len(r) {
					h.Sector = NormSector(r[isec])
				}
				if ictry >= 0 && ictry < len(r) {
					h.Country = NormCountry(r[ictry])
				}
				if h.Country == "" {
					h.Country = BloombergCountry[strings.ToUpper(code)]
				}
				holdings = append(holdings, h)
			}
			continue
		}
		if len(header) > 0 && holdingsHeadRE.MatchString(strings.TrimSpace(header[0])) {
			for _, r := range t[1:] {
				if len(r) < 2 {
					continue
				}
				nm, w := strings.TrimSpace(r[0]), numOr(r[1], 0)
				if w <= 0 || nm == "" || cashRowRE.MatchString(nm) {
					continue
				}
				holdings = append(holdings, Holding{Name: nm, Weight: w, Fund: IsFund(nm)})
			}
		}
	}
	return holdings, ref
}

func (c *Client) Harvest(symbol, name, exchange string) (*Breakdown, error) {
	doc, err := c.get(strings.Replace(HarvestPage, "%s", strings.ToLower(market.TMXSymbol(symbol)), 1), nil)
	if err != nil {
		return nil, err
	}
	holdings, ref := ParseHarvestTables(HTMLTables(doc))
	anyTicker := false
	for _, h := range holdings {
		if h.Ticker != "" {
			anyTicker = true
		}
	}
	if ref != "" && refTickerRE.MatchString(ref) && !anyTicker {
		holdings = []Holding{{Ticker: ref, Name: ref, Weight: 100.0}}
	}
	if len(holdings) == 0 {
		return nil, nil
	}
	return &Breakdown{Sectors: map[string]float64{}, Countries: map[string]float64{}, Holdings: holdings, Source: "Harvest ETFs"}, nil
}

const (
	NinepointList = "https://www.ninepoint.com/landing-pages/ninepoint-highshares-etfs/"
	NinepointBase = "https://www.ninepoint.com"
)

var tagRE = regexp.MustCompile(`<[^>]+>`)
var npTickerRE = regexp.MustCompile(`Ticker\s*\*?\*?\s*([A-Z0-9.]{1,8}):([A-Z]{2,6})\b`)
var npUnderRE = regexp.MustCompile(`Underlying Stock\s*\*?\*?\s*(.*?)\(([A-Z0-9.]{1,8}):([A-Z]{2,6})\)`)
var npSlugRE = regexp.MustCompile(`href="(?:https://www\.ninepoint\.com)?(/funds/[a-z0-9-]+/)"`)

func ParseNinepointPage(doc string) (string, string, string) {
	text := py.CollapseSpace(tagRE.ReplaceAllString(doc, " "))
	ticker, under, ex := "", "", ""
	if m := npTickerRE.FindStringSubmatch(text); m != nil {
		ticker = m[1]
	}
	if m := npUnderRE.FindStringSubmatch(text); m != nil {
		under, ex = m[2], m[3]
	}
	return ticker, under, ex
}

func (c *Client) ninepointSlugs() ([]string, error) {
	doc, err := c.get(NinepointList, nil)
	if err != nil {
		return nil, err
	}
	seen := map[string]bool{}
	var out []string
	for _, m := range npSlugRE.FindAllStringSubmatch(doc, -1) {
		if !seen[m[1]] {
			seen[m[1]] = true
			out = append(out, m[1])
		}
	}
	sort.Strings(out)
	return out, nil
}

func (c *Client) Ninepoint(symbol, name, exchange string) (*Breakdown, error) {
	sym := market.TMXSymbol(symbol)
	c.mu.Lock()
	_, known := c.ninepoint[sym]
	c.mu.Unlock()
	if !known {
		slugs, err := c.ninepointSlugs()
		if err != nil {
			return nil, err
		}
		for _, slug := range slugs {
			c.mu.Lock()
			_, have := c.ninepoint[sym]
			used := false
			for _, v := range c.ninepoint {
				if v == slug {
					used = true
				}
			}
			c.mu.Unlock()
			if have {
				break
			}
			if used {
				continue
			}
			doc, err := c.get(NinepointBase+slug, nil)
			if err != nil {
				continue
			}
			t, _, _ := ParseNinepointPage(doc)
			if t != "" {
				c.mu.Lock()
				c.ninepoint[t] = slug
				c.mu.Unlock()
			}
		}
	}
	c.mu.Lock()
	slug := c.ninepoint[sym]
	c.mu.Unlock()
	if slug == "" {
		return nil, nil
	}
	doc, err := c.get(NinepointBase+slug, nil)
	if err != nil {
		return nil, err
	}
	_, under, ex := ParseNinepointPage(doc)
	if under == "" {
		return nil, nil
	}
	return &Breakdown{Sectors: map[string]float64{}, Countries: map[string]float64{}, Holdings: []Holding{{Ticker: under, Name: under, Weight: 100.0, Exchange: ex}}, Source: "Ninepoint"}, nil
}

const EvolvePage = "https://evolveetfs.com/product/%s/"

var evolveBreakdownRE = regexp.MustCompile(`(?s)var portfolioBreakdownData\s*=\s*(\{.*?\});\s*\n`)
var evolveHoldingsRE = regexp.MustCompile(`(?s)var holdingsData\s*=\s*(\{.*?\});\s*\n`)

func ParseEvolvePage(doc string) (map[string]float64, []Holding) {
	sectors, holdings := map[string]float64{}, []Holding{}
	if m := evolveBreakdownRE.FindStringSubmatch(doc); m != nil {
		var d map[string]any
		if json.Unmarshal([]byte(m[1]), &d) == nil {
			data, _ := d["data"].(map[string]any)
			rows, _ := data["sector"].([]any)
			for _, raw := range rows {
				r, _ := raw.(map[string]any)
				n, w := NormSector(py.S(r["name"])), numOr(r["weight"], 0)
				if n != "" && w > 0 {
					sectors[n] += w
				}
			}
		}
	}
	if m := evolveHoldingsRE.FindStringSubmatch(doc); m != nil {
		var d map[string]any
		var rows []any
		if json.Unmarshal([]byte(m[1]), &d) == nil {
			rows, _ = d["data"].([]any)
		}
		for _, raw := range rows {
			r, _ := raw.(map[string]any)
			tk := strings.TrimSpace(py.S(r["ticker"]))
			parts := py.Fields(tk)
			sym, code := tk, ""
			if len(parts) >= 2 {
				sym, code = parts[0], parts[1]
			}
			w := numOr(r["weight_percent"], 0)
			nm := strings.TrimSpace(py.S(r["security_name"]))
			if w <= 0 || sym == "" {
				continue
			}
			ctry := NormCountry(py.S(r["country"]))
			if ctry != "" && len(ctry) <= 5 && strings.ToUpper(ctry) == ctry {
				ctry = ""
			}
			if ctry == "" {
				ctry = BloombergCountry[strings.ToUpper(code)]
			}
			holdings = append(holdings, Holding{Ticker: sym, Name: nm, Weight: w, Sector: NormSector(py.S(r["gics_sector"])), Country: ctry, Fund: IsFund(nm)})
		}
	}
	return sectors, holdings
}

func (c *Client) Evolve(symbol, name, exchange string) (*Breakdown, error) {
	doc, err := c.get(strings.Replace(EvolvePage, "%s", strings.ToLower(market.TMXSymbol(symbol)), 1), nil)
	if err != nil {
		return nil, err
	}
	sectors, holdings := ParseEvolvePage(doc)
	if len(sectors) == 0 && len(holdings) == 0 {
		return nil, nil
	}
	return &Breakdown{Sectors: sectors, Countries: map[string]float64{}, Holdings: holdings, Source: "Evolve ETFs"}, nil
}

const (
	YahooCrumb   = "https://query2.finance.yahoo.com/v1/test/getcrumb"
	YahooSummary = "https://query2.finance.yahoo.com/v10/finance/quoteSummary/%s?modules=topHoldings&crumb=%s"
)

var YahooSuffix = map[string]string{"TSX": ".TO", "TSX-V": ".V", "TSXV": ".V", "CSE": ".CN", "CBOE CANADA": ".NE", "NEO": ".NE"}

func (c *Client) yahooSession() (string, string, error) {
	c.mu.Lock()
	if c.yahooCrumb != "" {
		cookie, crumb := c.yahooCookie, c.yahooCrumb
		c.mu.Unlock()
		return cookie, crumb, nil
	}
	c.mu.Unlock()
	c.pacer.Pace("fc.yahoo.com", PaceSec)
	var cookies []string
	req, _ := http.NewRequest(http.MethodGet, "https://fc.yahoo.com", nil)
	req.Header.Set("User-Agent", UA)
	client := &http.Client{Transport: c.Market.HTTP.Transport, Timeout: market.TimeoutSec * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	if resp, err := client.Do(req); err == nil {
		for _, v := range resp.Header.Values("Set-Cookie") {
			cookies = append(cookies, strings.Split(v, ";")[0])
		}
		resp.Body.Close()
	}
	cookie := strings.Join(cookies, "; ")
	crumb, err := c.get(YahooCrumb, map[string]string{"Cookie": cookie})
	if err != nil {
		return "", "", err
	}
	crumb = strings.TrimSpace(crumb)
	c.mu.Lock()
	c.yahooCookie, c.yahooCrumb = cookie, crumb
	c.mu.Unlock()
	return cookie, crumb, nil
}

func YahooSymbol(symbol, exchange string) string {
	return market.TMXSymbol(symbol) + YahooSuffix[strings.ToUpper(strings.TrimSpace(exchange))]
}

func rawOf(v any) float64 {
	if m, ok := v.(map[string]any); ok {
		return numOr(m["raw"], 0)
	}
	return numOr(v, 0)
}

func ParseYahooSummary(data map[string]any) (map[string]float64, []Holding) {
	qs, _ := data["quoteSummary"].(map[string]any)
	results, _ := qs["result"].([]any)
	res := map[string]any{}
	if len(results) > 0 {
		res, _ = results[0].(map[string]any)
	}
	th, _ := res["topHoldings"].(map[string]any)
	sectors := map[string]float64{}
	weights, _ := th["sectorWeightings"].([]any)
	for _, raw := range weights {
		entry, _ := raw.(map[string]any)
		for _, k := range sortedKeysAny(entry) {
			w := rawOf(entry[k])
			n := NormSector(strings.ReplaceAll(k, "_", " "))
			if n != "" && w > 0 {
				sectors[n] = py.Round(sectors[n]+w*100.0, 4)
			}
		}
	}
	holdings := []Holding{}
	rows, _ := th["holdings"].([]any)
	for _, raw := range rows {
		h, _ := raw.(map[string]any)
		sym := strings.TrimSpace(py.S(h["symbol"]))
		w := rawOf(h["holdingPercent"])
		if sym != "" && w > 0 {
			ex := ""
			for _, sv := range []struct{ suf, venue string }{{".TO", "TSX"}, {".V", "TSX-V"}, {".CN", "CSE"}, {".NE", "CBOE CANADA"}} {
				if strings.HasSuffix(strings.ToUpper(sym), sv.suf) {
					ex = sv.venue
				}
			}
			ccy := ""
			if ex != "" {
				ccy = "CAD"
			}
			holdings = append(holdings, Holding{Ticker: sym, Name: py.S(h["holdingName"]), Weight: py.Round(w*100.0, 4), Exchange: ex, Currency: ccy, Fund: IsFund(py.S(h["holdingName"]))})
		}
	}
	return sectors, holdings
}

func sortedKeysAny(m map[string]any) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func (c *Client) YahooFund(symbol, name, exchange string) (*Breakdown, error) {
	cookie, crumb, err := c.yahooSession()
	if err != nil {
		return nil, err
	}
	raw, err := c.get(strings.Replace(strings.Replace(YahooSummary, "%s", YahooSymbol(symbol, exchange), 1), "%s", crumb, 1), map[string]string{"Cookie": cookie, "Accept": "application/json"})
	if err != nil {
		return nil, err
	}
	var d map[string]any
	if err := json.Unmarshal([]byte(raw), &d); err != nil {
		return nil, err
	}
	sectors, holdings := ParseYahooSummary(d)
	if len(sectors) == 0 && len(holdings) == 0 {
		return nil, nil
	}
	return &Breakdown{Sectors: sectors, Countries: map[string]float64{}, Holdings: holdings, Source: "Yahoo Finance"}, nil
}

type adapter func(symbol, name, exchange string) (*Breakdown, error)

func (c *Client) adapters() map[string]adapter {
	return map[string]adapter{"vanguard": c.VanguardCA, "ishares": c.ISharesCA, "harvest": c.Harvest, "ninepoint": c.Ninepoint, "evolve": c.Evolve}
}

var nameNoiseRE = regexp.MustCompile(`(?i)\b(inc|corp|corporation|ltd|limited|plc|co|class [a-z]|common shares?|common stock|the)\b\.?`)
var nameKeepRE = regexp.MustCompile(`[^A-Za-z0-9 &.-]`)

func (c *Client) ResolveName(name string) *SearchMatch {
	if c.SymbolSearch == nil {
		return nil
	}
	clean := nameNoiseRE.ReplaceAllString(name, " ")
	clean = nameKeepRE.ReplaceAllString(clean, " ")
	clean = py.Strip(py.CollapseSpace(clean))
	if clean == "" {
		return nil
	}
	if len(clean) > 40 {
		clean = clean[:40]
	}
	matches := c.SymbolSearch(clean)
	if len(matches) == 0 {
		return nil
	}
	m := matches[0]
	return &m
}

func (c *Client) cacheGet(key string) *store.Exposure {
	rec := c.Store.ExposureRecord(key)
	if rec == nil {
		return nil
	}
	t, err := time.Parse("2006-01-02T15:04:05Z", rec.FetchedAt)
	if err != nil {
		return nil
	}
	age := int(time.Now().UTC().Sub(t).Hours() / 24)
	if age < FreshDays {
		return rec
	}
	return nil
}

func (c *Client) ShareExposure(symbol, exchange, currency string) store.Exposure {
	key := ShareKey + market.TMXSymbol(symbol) + ":" + market.TMXFormOr(exchange, currency)
	if hit := c.cacheGet(key); hit != nil {
		return *hit
	}
	cl := c.ClassifyShare(symbol, exchange, currency)
	rec := store.Exposure{Sectors: map[string]float64{}, Countries: map[string]float64{}, Source: cl.Source, Industry: cl.Industry}
	if cl.Sector != "" {
		rec.Sectors[cl.Sector] = 1.0
	}
	if cl.Country != "" {
		rec.Countries[cl.Country] = 1.0
	}
	if cl.Sector != "" || cl.Country != "" {
		rec.Coverage = 1.0
	}
	c.Store.ReplaceExposure(key, rec)
	return rec
}

func firstKey(m map[string]float64) string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	if len(keys) == 0 {
		return ""
	}
	return keys[0]
}

func (c *Client) Lookthrough(holdings []Holding, depth int, seen map[string]bool) store.Exposure {
	if seen == nil {
		seen = map[string]bool{}
	}
	var rows []Holding
	total := 0.0
	for _, h := range holdings {
		if h.Weight > 0 {
			rows = append(rows, h)
			total += h.Weight
		}
	}
	sectors, countries := map[string]float64{}, map[string]float64{}
	covered := 0.0
	if total <= 0 {
		return store.Exposure{Sectors: sectors, Countries: countries}
	}
	for _, h := range rows {
		w := h.Weight / total
		sec, ctry := h.Sector, h.Country
		tk, ex, ccy := h.Ticker, h.Exchange, h.Currency
		if sec != "" && ctry != "" {
			sectors[sec] += w
			countries[ctry] += w
			covered += w
			continue
		}
		if tk == "" && h.Name != "" {
			if m := c.ResolveName(h.Name); m != nil {
				tk, ex, ccy = m.Symbol, m.Exchange, m.Currency
			}
		}
		var sub *store.Exposure
		if h.Fund && depth < MaxDepth && (tk != "" || h.Name != "") {
			sub = c.FundExposure(tk, h.Name, ex, depth+1, seen)
		}
		if sub != nil && (len(sub.Sectors) > 0 || len(sub.Countries) > 0) {
			for n, f := range sub.Sectors {
				sectors[n] += w * f
			}
			for n, f := range sub.Countries {
				countries[n] += w * f
			}
			covered += w * sub.Coverage
			continue
		}
		if tk != "" {
			if ex == "" && ctry == "United States" && ccy == "" {
				ccy = "USD"
			}
			cl := c.ShareExposure(tk, ex, ccy)
			if sec == "" {
				sec = firstKey(cl.Sectors)
			}
			if ctry == "" {
				ctry = firstKey(cl.Countries)
			}
		}
		if sec != "" {
			sectors[sec] += w
		}
		if ctry != "" {
			countries[ctry] += w
		}
		if sec != "" || ctry != "" {
			covered += w
		}
	}
	if covered > 1 {
		covered = 1
	}
	return store.Exposure{Sectors: sectors, Countries: countries, Coverage: covered}
}

func (c *Client) FundExposure(symbol, name, exchange string, depth int, seen map[string]bool) *store.Exposure {
	if seen == nil {
		seen = map[string]bool{}
	}
	sym := symbol
	if sym == "" {
		sym = name
	}
	key := FundKey + market.TMXSymbol(sym)
	if seen[key] {
		return nil
	}
	seen[key] = true
	if hit := c.cacheGet(key); hit != nil {
		return hit
	}
	family := IssuerOf(name)
	var data *Breakdown
	if ad, ok := c.adapters()[family]; ok {
		d, err := ad(symbol, name, exchange)
		if err != nil {
			c.Market.NoteSource(family, false, err)
		} else {
			data = d
		}
	}
	if data == nil {
		d, err := c.YahooFund(symbol, name, exchange)
		if err != nil {
			c.Market.NoteSource("yahoo", false, err)
		} else {
			data = d
		}
	}
	if data == nil {
		return nil
	}
	sectors, countries := map[string]float64{}, map[string]float64{}
	for n, w := range data.Sectors {
		sectors[n] = w / 100.0
	}
	for n, w := range data.Countries {
		countries[n] = w / 100.0
	}
	coverage := 0.0
	if len(sectors) > 0 || len(countries) > 0 {
		coverage = 1.0
	}
	if len(data.Holdings) > 0 && (len(sectors) == 0 || len(countries) == 0) {
		agg := c.Lookthrough(data.Holdings, depth, seen)
		if len(sectors) == 0 {
			sectors = agg.Sectors
		}
		if len(countries) == 0 {
			countries = agg.Countries
		}
		if len(sectors) > 0 && len(countries) > 0 {
			if agg.Coverage > coverage {
				coverage = agg.Coverage
			}
		} else {
			coverage = agg.Coverage
		}
	}
	totS, totC := 0.0, 0.0
	for _, w := range sectors {
		totS += w
	}
	for _, w := range countries {
		totC += w
	}
	if totS > 1.0001 {
		for n, w := range sectors {
			sectors[n] = w / totS
		}
	}
	if totC > 1.0001 {
		for n, w := range countries {
			countries[n] = w / totC
		}
	}
	source := data.Source
	if source == "" {
		source = family
	}
	rec := store.Exposure{Sectors: sectors, Countries: countries, Coverage: coverage, Source: source, AsOf: data.AsOf}
	c.Store.ReplaceExposure(key, rec)
	return &rec
}

func (c *Client) RefreshSecurity(sec store.Security) store.Exposure {
	var rec store.Exposure
	func() {
		defer func() {
			if e := recover(); e != nil {
				rec = store.Exposure{Sectors: map[string]float64{}, Countries: map[string]float64{}, Error: py.S(e)}
			}
		}()
		if IsFund(sec.Name) {
			if fund := c.FundExposure(sec.Symbol, sec.Name, sec.PrimaryExchange, 0, nil); fund != nil {
				rec = *fund
			} else {
				rec = store.Exposure{Sectors: map[string]float64{}, Countries: map[string]float64{}}
			}
		} else {
			rec = c.ShareExposure(sec.Symbol, sec.PrimaryExchange, sec.Currency)
		}
	}()
	c.Store.ReplaceExposure(sec.ID, rec)
	return rec
}

func (c *Client) Stale(securityIDs []string) []string {
	out := []string{}
	for _, sid := range securityIDs {
		if c.cacheGet(sid) == nil {
			out = append(out, sid)
		}
	}
	return out
}
