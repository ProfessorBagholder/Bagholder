package fear

import (
	"fmt"
	"os"
	"sort"
	"strings"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/market"
	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const (
	StockURL  = "https://production.dataviz.cnn.io/index/fearandgreed/graphdata"
	CryptoURL = "https://api.alternative.me/fng/?limit=%d"
	Days      = 366
)

var StockHeaders = map[string]string{"User-Agent": market.UA, "Accept": "application/json", "Origin": "https://www.cnn.com", "Referer": "https://www.cnn.com/"}
var CryptoHeaders = map[string]string{"User-Agent": market.UA, "Accept": "application/json"}
var Indexes = []string{"stocks", "crypto"}
var Sources = map[string]string{"stocks": "CNN", "crypto": "Alternative.me"}

var Parts = []struct{ Key, Name string }{
	{"market_momentum_sp125", "Market momentum"},
	{"stock_price_strength", "Stock price strength"},
	{"stock_price_breadth", "Stock price breadth"},
	{"put_call_options", "Put and call options"},
	{"market_volatility_vix_50", "Market volatility"},
	{"junk_bond_demand", "Junk bond demand"},
	{"safe_haven_demand", "Safe haven demand"},
}

var Bands = []struct {
	Edge float64
	Name string
}{{25, "Extreme fear"}, {45, "Fear"}, {56, "Neutral"}, {76, "Greed"}}

func Band(score float64) string {
	for _, b := range Bands {
		if score < b.Edge {
			return b.Name
		}
	}
	return "Extreme greed"
}

func Rating(given string, score float64) string {
	text := strings.TrimSpace(given)
	if text == "" {
		return Band(score)
	}
	return strings.ToUpper(text[:1]) + strings.ToLower(text[1:])
}

func day(ms float64) string {
	sec := ms / 1000.0
	return time.Unix(int64(sec), int64((sec-float64(int64(sec)))*1e9)).UTC().Format("2006-01-02")
}

func moment(text string) string {
	t, naive, ok := py.ParseISO(text)
	if !ok {
		return ""
	}
	if naive {
		t = time.Date(t.Year(), t.Month(), t.Day(), t.Hour(), t.Minute(), t.Second(), t.Nanosecond(), time.Local)
	}
	return t.UTC().Format("2006-01-02T15:04:05Z")
}

func reading(label string, score py.JSONNum) map[string]any {
	if !score.OK {
		return nil
	}
	return map[string]any{"label": label, "score": py.Round(score.F, 1), "rating": Band(score.F)}
}

type indicator struct {
	Score  py.JSONNum  `json:"score"`
	Rating py.JSONText `json:"rating"`
}

type Stocks struct {
	FearAndGreed struct {
		Score          py.JSONNum  `json:"score"`
		Rating         py.JSONText `json:"rating"`
		Timestamp      py.JSONText `json:"timestamp"`
		PreviousClose  py.JSONNum  `json:"previous_close"`
		Previous1Week  py.JSONNum  `json:"previous_1_week"`
		Previous1Month py.JSONNum  `json:"previous_1_month"`
		Previous1Year  py.JSONNum  `json:"previous_1_year"`
	} `json:"fear_and_greed"`
	Historical struct {
		Data []py.JSONLoose[struct {
			X py.JSONNum `json:"x"`
			Y py.JSONNum `json:"y"`
		}] `json:"data"`
	} `json:"fear_and_greed_historical"`
	Momentum   py.JSONLoose[indicator] `json:"market_momentum_sp125"`
	Strength   py.JSONLoose[indicator] `json:"stock_price_strength"`
	Breadth    py.JSONLoose[indicator] `json:"stock_price_breadth"`
	PutCall    py.JSONLoose[indicator] `json:"put_call_options"`
	Volatility py.JSONLoose[indicator] `json:"market_volatility_vix_50"`
	JunkBonds  py.JSONLoose[indicator] `json:"junk_bond_demand"`
	SafeHaven  py.JSONLoose[indicator] `json:"safe_haven_demand"`
}

func (s Stocks) indicators() []indicator {
	return []indicator{s.Momentum.V, s.Strength.V, s.Breadth.V, s.PutCall.V, s.Volatility.V, s.JunkBonds.V, s.SafeHaven.V}
}

func ParseStocks(data Stocks) map[string]any {
	fg := data.FearAndGreed
	if !fg.Score.OK {
		return map[string]any{}
	}
	score := fg.Score.F
	earlier := []map[string]any{reading("Previous close", fg.PreviousClose), reading("A week ago", fg.Previous1Week), reading("A month ago", fg.Previous1Month), reading("A year ago", fg.Previous1Year)}
	parts := []map[string]any{}
	indicators := data.indicators()
	for i, p := range Parts {
		part := indicators[i]
		if part.Score.OK {
			parts = append(parts, map[string]any{"name": p.Name, "score": py.Round(part.Score.F, 1), "rating": Rating(string(part.Rating), part.Score.F)})
		}
	}
	series := []map[string]any{}
	for _, raw := range data.Historical.Data {
		point := raw.V
		if !point.X.OK || !point.Y.OK {
			continue
		}
		if d := day(point.X.F); d != "" {
			series = append(series, map[string]any{"date": d, "score": py.Round(point.Y.F, 1)})
		}
	}
	sort.SliceStable(series, func(i, j int) bool { return series[i]["date"].(string) < series[j]["date"].(string) })
	previous := []map[string]any{}
	for _, r := range earlier {
		if r != nil {
			previous = append(previous, r)
		}
	}
	return map[string]any{"index": "stocks", "source": Sources["stocks"], "score": py.Round(score, 1), "rating": Rating(string(fg.Rating), score), "asOf": moment(string(fg.Timestamp)), "previous": previous, "parts": parts, "series": series}
}

type Crypto struct {
	Data []py.JSONLoose[struct {
		Value          py.JSONNum  `json:"value"`
		Classification py.JSONText `json:"value_classification"`
		Timestamp      py.JSONNum  `json:"timestamp"`
	}] `json:"data"`
}

func ParseCrypto(data Crypto) map[string]any {
	rows := []map[string]any{}
	for _, raw := range data.Data {
		row := raw.V
		var d string
		if row.Timestamp.OK {
			d = day(row.Timestamp.F * 1000)
		}
		if d != "" && row.Value.OK {
			rows = append(rows, map[string]any{"date": d, "score": py.Round(row.Value.F, 1), "rating": Rating(string(row.Classification), row.Value.F)})
		}
	}
	if len(rows) == 0 {
		return map[string]any{}
	}
	now := rows[0]
	at := func(i int, label string) map[string]any {
		if len(rows) > i {
			return map[string]any{"label": label, "score": rows[i]["score"], "rating": rows[i]["rating"]}
		}
		return nil
	}
	previous := []map[string]any{}
	for _, r := range []map[string]any{at(1, "Yesterday"), at(7, "A week ago"), at(30, "A month ago"), at(365, "A year ago")} {
		if r != nil {
			previous = append(previous, r)
		}
	}
	series := []map[string]any{}
	for _, r := range rows {
		series = append(series, map[string]any{"date": r["date"], "score": r["score"]})
	}
	sort.SliceStable(series, func(i, j int) bool { return series[i]["date"].(string) < series[j]["date"].(string) })
	return map[string]any{"index": "crypto", "source": Sources["crypto"], "score": now["score"], "rating": now["rating"], "asOf": now["date"].(string) + "T00:00:00Z", "previous": previous, "parts": []map[string]any{}, "series": series}
}

func Read(c *market.Client, index string) map[string]any {
	which := strings.ToLower(strings.TrimSpace(index))
	var url string
	var headers map[string]string
	switch which {
	case "stocks":
		url, headers = StockURL, StockHeaders
	case "crypto":
		url, headers = fmt.Sprintf(CryptoURL, Days), CryptoHeaders
	default:
		return map[string]any{}
	}
	var stocks Stocks
	var crypto Crypto
	var err error
	if which == "stocks" {
		err = c.GetJSON(url, headers, &stocks)
	} else {
		err = c.GetJSON(url, headers, &crypto)
	}
	if err != nil {
		src := Sources[which]
		if src == "" {
			src = "its publisher"
		}
		fmt.Fprintf(os.Stderr, "bagholder fear: %s from %s failed: %s\n", which, src, err)
		return map[string]any{}
	}
	if which == "stocks" {
		return ParseStocks(stocks)
	}
	return ParseCrypto(crypto)
}
