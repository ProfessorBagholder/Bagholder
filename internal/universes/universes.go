package universes

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/exposure"
	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const (
	CanadaIndex          = "^TX60"
	Top                  = 100
	TMXConstituentsQuery = "query getIndexConstituents($symbol: String!) { constituents: getIndexConstituents(symbol: $symbol) { symbol quotedMarketValue longName shortName weight exShortName exchange exLongName } }"
	TMXTileQuery         = "query getQuoteBySymbol($symbol: String, $locale: String) { getQuoteBySymbol(symbol: $symbol, locale: $locale) { symbol name price percentChange sector } }"
	ScreenerURL          = "https://api.nasdaq.com/api/screener/stocks?tableonly=true&limit=25&offset=0&download=true"
	UA                   = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0 Safari/537.36"
)

var NasdaqHeaders = map[string]string{"User-Agent": UA, "Accept": "application/json, text/plain, */*", "Origin": "https://www.nasdaq.com", "Referer": "https://www.nasdaq.com/"}
var TMXHeaders = map[string]string{"User-Agent": UA, "locale": "en", "Origin": "https://money.tmx.com", "Referer": "https://money.tmx.com/"}
var Keys = []string{"ca", "us", "intl"}

var pacer = market.NewPacer()

func num(v any) *float64 {
	if v == nil {
		return nil
	}
	s := strings.TrimSpace(strings.NewReplacer("$", "", "%", "", ",", "").Replace(py.S(v)))
	if s == "" || s == "N/A" || s == "NA" || s == "None" {
		return nil
	}
	f, ok := py.NumOK(s)
	if !ok {
		return nil
	}
	return &f
}

func SectorOf(name string) string {
	if s := exposure.NormSector(name); s != "" {
		return s
	}
	return "Not classified"
}

type ScreenerRow struct {
	Symbol        string
	Name          string
	Last          *float64
	PercentChange *float64
	Cap           float64
	Sector        string
	Country       string
}

func ParseScreener(data map[string]any) []ScreenerRow {
	d, _ := data["data"].(map[string]any)
	rows, _ := d["rows"].([]any)
	out := []ScreenerRow{}
	for _, raw := range rows {
		r, ok := raw.(map[string]any)
		if !ok || py.S(r["symbol"]) == "" {
			continue
		}
		out = append(out, ScreenerRow{Symbol: strings.TrimSpace(py.S(r["symbol"])), Name: strings.TrimSpace(py.S(r["name"])), Last: num(r["lastsale"]), PercentChange: num(r["pctchange"]), Cap: py.Deref(num(r["marketCap"]), 0), Sector: SectorOf(py.S(r["sector"])), Country: strings.TrimSpace(py.S(r["country"]))})
	}
	return out
}

func pick(rows []ScreenerRow, n int, keep func(r ScreenerRow) bool) []store.Universe {
	var picked []ScreenerRow
	for _, r := range rows {
		if keep(r) && r.Cap > 0 {
			picked = append(picked, r)
		}
	}
	sort.SliceStable(picked, func(i, j int) bool { return picked[i].Cap > picked[j].Cap })
	if len(picked) > n {
		picked = picked[:n]
	}
	out := []store.Universe{}
	for _, r := range picked {
		out = append(out, store.Universe{Symbol: r.Symbol, Name: r.Name, Value: py.Ptr(r.Cap), PercentChange: r.PercentChange, Sector: r.Sector, Country: r.Country})
	}
	return out
}

func USRows(rows []ScreenerRow, n int) []store.Universe {
	return pick(rows, n, func(r ScreenerRow) bool { return r.Country == "United States" })
}

func IntlRows(rows []ScreenerRow, n int) []store.Universe {
	return pick(rows, n, func(r ScreenerRow) bool { return r.Country != "United States" && r.Country != "Canada" && r.Country != "" })
}

type Constituent struct {
	Symbol   string
	Name     string
	Weight   float64
	Cap      float64
	Exchange string
}

func ParseConstituents(data map[string]any) []Constituent {
	d, _ := data["data"].(map[string]any)
	rows, _ := d["constituents"].([]any)
	out := []Constituent{}
	for _, raw := range rows {
		c, ok := raw.(map[string]any)
		if !ok || py.S(c["symbol"]) == "" {
			continue
		}
		name := py.S(c["longName"])
		if name == "" {
			name = py.S(c["shortName"])
		}
		out = append(out, Constituent{Symbol: strings.TrimSpace(py.S(c["symbol"])), Name: strings.TrimSpace(name), Weight: py.Deref(num(c["weight"]), 0), Cap: py.Deref(num(c["quotedMarketValue"]), 0), Exchange: strings.TrimSpace(py.S(c["exchange"]))})
	}
	return out
}

type TileQuote struct {
	PercentChange *float64
	Sector        string
	Name          string
}

func ParseTileQuote(data map[string]any) *TileQuote {
	d, _ := data["data"].(map[string]any)
	q, _ := d["getQuoteBySymbol"].(map[string]any)
	if len(q) == 0 {
		return nil
	}
	return &TileQuote{PercentChange: num(q["percentChange"]), Sector: SectorOf(py.S(q["sector"])), Name: strings.TrimSpace(py.S(q["name"]))}
}

func FetchScreener(c *market.Client) ([]ScreenerRow, error) {
	pacer.Pace("api.nasdaq.com", 0.6)
	text, err := c.GetText(ScreenerURL, NasdaqHeaders)
	if err != nil {
		return nil, err
	}
	var data map[string]any
	if err := json.Unmarshal([]byte(text), &data); err != nil {
		return nil, err
	}
	return ParseScreener(data), nil
}

func FetchCanada(c *market.Client) ([]store.Universe, error) {
	pacer.Pace("app-money.tmx.com", 0.6)
	data, err := c.PostJSON(market.TMXURL, map[string]any{"operationName": "getIndexConstituents", "variables": map[string]any{"symbol": CanadaIndex}, "query": TMXConstituentsQuery}, TMXHeaders)
	if err != nil {
		return nil, err
	}
	out := []store.Universe{}
	for _, con := range ParseConstituents(data) {
		var q *TileQuote
		pacer.Pace("app-money.tmx.com", 0.6)
		qd, err := c.PostJSON(market.TMXURL, map[string]any{"operationName": "getQuoteBySymbol", "variables": map[string]any{"symbol": con.Symbol, "locale": "en"}, "query": TMXTileQuery}, TMXHeaders)
		if err != nil {
			fmt.Fprintf(os.Stderr, "bagholder universes: %s quote failed: %s\n", con.Symbol, err)
		} else {
			q = ParseTileQuote(qd)
		}
		value := con.Weight
		if value == 0 {
			value = con.Cap
		}
		row := store.Universe{Symbol: con.Symbol, Name: con.Name, Value: py.Ptr(value), Sector: "Not classified", Country: "Canada"}
		if q != nil {
			row.PercentChange = q.PercentChange
			row.Sector = q.Sector
		}
		out = append(out, row)
	}
	return out, nil
}

func Refresh(c *market.Client) []string {
	done := []string{}
	rows, err := FetchScreener(c)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder universes: Nasdaq's screener failed: %s\n", err)
	} else {
		c.Store.ReplaceUniverse("us", USRows(rows, Top), "")
		c.Store.ReplaceUniverse("intl", IntlRows(rows, Top), "")
		done = append(done, "us", "intl")
	}
	ca, err := FetchCanada(c)
	if err != nil {
		fmt.Fprintf(os.Stderr, "bagholder universes: the S&P/TSX 60 failed: %s\n", err)
	} else if len(ca) > 0 {
		c.Store.ReplaceUniverse("ca", ca, "")
		done = append(done, "ca")
	}
	return done
}
