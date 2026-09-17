package universes

import (
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

func num(v py.JSONText) *float64 {
	s := strings.TrimSpace(strings.NewReplacer("$", "", "%", "", ",", "").Replace(string(v)))
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

type Screener struct {
	Data struct {
		Rows []py.JSONLoose[struct {
			Symbol    py.JSONText `json:"symbol"`
			Name      py.JSONText `json:"name"`
			LastSale  py.JSONText `json:"lastsale"`
			PctChange py.JSONText `json:"pctchange"`
			MarketCap py.JSONText `json:"marketCap"`
			Sector    py.JSONText `json:"sector"`
			Country   py.JSONText `json:"country"`
		}] `json:"rows"`
	} `json:"data"`
}

func ParseScreener(data Screener) []ScreenerRow {
	out := []ScreenerRow{}
	for _, raw := range data.Data.Rows {
		r := raw.V
		if r.Symbol == "" {
			continue
		}
		out = append(out, ScreenerRow{Symbol: strings.TrimSpace(string(r.Symbol)), Name: strings.TrimSpace(string(r.Name)), Last: num(r.LastSale), PercentChange: num(r.PctChange), Cap: py.Deref(num(r.MarketCap), 0), Sector: SectorOf(string(r.Sector)), Country: strings.TrimSpace(string(r.Country))})
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
	return pick(rows, n, func(r ScreenerRow) bool {
		return r.Country != "United States" && r.Country != "Canada" && r.Country != ""
	})
}

type Constituent struct {
	Symbol   string
	Name     string
	Weight   float64
	Cap      float64
	Exchange string
}

type Constituents struct {
	Data struct {
		Constituents []py.JSONLoose[struct {
			Symbol            py.JSONText `json:"symbol"`
			LongName          py.JSONText `json:"longName"`
			ShortName         py.JSONText `json:"shortName"`
			Weight            py.JSONText `json:"weight"`
			QuotedMarketValue py.JSONText `json:"quotedMarketValue"`
			Exchange          py.JSONText `json:"exchange"`
		}] `json:"constituents"`
	} `json:"data"`
}

func ParseConstituents(data Constituents) []Constituent {
	out := []Constituent{}
	for _, raw := range data.Data.Constituents {
		c := raw.V
		if c.Symbol == "" {
			continue
		}
		name := string(c.LongName)
		if name == "" {
			name = string(c.ShortName)
		}
		out = append(out, Constituent{Symbol: strings.TrimSpace(string(c.Symbol)), Name: strings.TrimSpace(name), Weight: py.Deref(num(c.Weight), 0), Cap: py.Deref(num(c.QuotedMarketValue), 0), Exchange: strings.TrimSpace(string(c.Exchange))})
	}
	return out
}

type TileQuote struct {
	PercentChange *float64
	Sector        string
	Name          string
}

type Tile struct {
	Data struct {
		GetQuoteBySymbol map[string]py.JSONText `json:"getQuoteBySymbol"`
	} `json:"data"`
}

func ParseTileQuote(data Tile) *TileQuote {
	q := data.Data.GetQuoteBySymbol
	if len(q) == 0 {
		return nil
	}
	return &TileQuote{PercentChange: num(q["percentChange"]), Sector: SectorOf(string(q["sector"])), Name: strings.TrimSpace(string(q["name"]))}
}

func FetchScreener(c *market.Client) ([]ScreenerRow, error) {
	pacer.Pace("api.nasdaq.com", 0.6)
	var data Screener
	if err := c.GetJSON(ScreenerURL, NasdaqHeaders, &data); err != nil {
		return nil, err
	}
	return ParseScreener(data), nil
}

func FetchCanada(c *market.Client) ([]store.Universe, error) {
	pacer.Pace("app-money.tmx.com", 0.6)
	var data Constituents
	if err := c.PostJSONInto(market.TMXURL, map[string]any{"operationName": "getIndexConstituents", "variables": map[string]any{"symbol": CanadaIndex}, "query": TMXConstituentsQuery}, TMXHeaders, &data); err != nil {
		return nil, err
	}
	out := []store.Universe{}
	for _, con := range ParseConstituents(data) {
		var q *TileQuote
		pacer.Pace("app-money.tmx.com", 0.6)
		var qd Tile
		err := c.PostJSONInto(market.TMXURL, map[string]any{"operationName": "getQuoteBySymbol", "variables": map[string]any{"symbol": con.Symbol, "locale": "en"}, "query": TMXTileQuery}, TMXHeaders, &qd)
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
