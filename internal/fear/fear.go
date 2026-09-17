package fear

import (
	"encoding/json"
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

func num(v any) *float64 {
	f, ok := py.NumOK(v)
	if !ok {
		return nil
	}
	return &f
}

func Band(score any) string {
	n := num(score)
	if n == nil {
		return ""
	}
	for _, b := range Bands {
		if *n < b.Edge {
			return b.Name
		}
	}
	return "Extreme greed"
}

func Rating(given any, score any) string {
	text := strings.TrimSpace(py.S(given))
	if text == "" {
		return Band(score)
	}
	return strings.ToUpper(text[:1]) + strings.ToLower(text[1:])
}

func day(ms any) string {
	n := num(ms)
	if n == nil {
		return ""
	}
	sec := *n / 1000.0
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

func reading(label string, score any) map[string]any {
	n := num(score)
	if n == nil {
		return nil
	}
	return map[string]any{"label": label, "score": py.Round(*n, 1), "rating": Band(*n)}
}

func ParseStocks(data map[string]any) map[string]any {
	fg, _ := data["fear_and_greed"].(map[string]any)
	score := num(fg["score"])
	if score == nil {
		return map[string]any{}
	}
	earlier := []map[string]any{reading("Previous close", fg["previous_close"]), reading("A week ago", fg["previous_1_week"]), reading("A month ago", fg["previous_1_month"]), reading("A year ago", fg["previous_1_year"])}
	parts := []map[string]any{}
	for _, p := range Parts {
		part, _ := data[p.Key].(map[string]any)
		if value := num(part["score"]); value != nil {
			parts = append(parts, map[string]any{"name": p.Name, "score": py.Round(*value, 1), "rating": Rating(part["rating"], *value)})
		}
	}
	series := []map[string]any{}
	hist, _ := data["fear_and_greed_historical"].(map[string]any)
	points, _ := hist["data"].([]any)
	for _, raw := range points {
		point, _ := raw.(map[string]any)
		d, value := day(point["x"]), num(point["y"])
		if d != "" && value != nil {
			series = append(series, map[string]any{"date": d, "score": py.Round(*value, 1)})
		}
	}
	sort.SliceStable(series, func(i, j int) bool { return series[i]["date"].(string) < series[j]["date"].(string) })
	previous := []map[string]any{}
	for _, r := range earlier {
		if r != nil {
			previous = append(previous, r)
		}
	}
	return map[string]any{"index": "stocks", "source": Sources["stocks"], "score": py.Round(*score, 1), "rating": Rating(fg["rating"], *score), "asOf": moment(py.S(fg["timestamp"])), "previous": previous, "parts": parts, "series": series}
}

func ParseCrypto(data map[string]any) map[string]any {
	items, _ := data["data"].([]any)
	rows := []map[string]any{}
	for _, raw := range items {
		row, _ := raw.(map[string]any)
		var d string
		if ts := num(row["timestamp"]); ts != nil {
			d = day(*ts * 1000)
		}
		value := num(row["value"])
		if d != "" && value != nil {
			rows = append(rows, map[string]any{"date": d, "score": py.Round(*value, 1), "rating": Rating(row["value_classification"], *value)})
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
	text, err := c.GetText(url, headers)
	var data map[string]any
	if err == nil {
		err = json.Unmarshal([]byte(text), &data)
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
		return ParseStocks(data)
	}
	return ParseCrypto(data)
}
