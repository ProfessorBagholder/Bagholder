package fear

import (
	"encoding/json"
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const fearVersion = 1

const cnnJSON = `{
	"fear_and_greed": {"score": 28.6571428571429, "rating": "fear", "timestamp": "2026-09-15T23:59:51+00:00",
		"previous_close": 31.0571428571429, "previous_1_week": 39.142857142857146,
		"previous_1_month": 64.31428571428572, "previous_1_year": 64.45714285714287},
	"fear_and_greed_historical": {"data": [{"x": 1789516791000.0, "y": 28.6571428571429, "rating": "fear"},
		{"x": 1757980800000.0, "y": 64.37142857142858, "rating": "greed"}]},
	"market_momentum_sp125": {"score": 22.8, "rating": "extreme fear", "data": []},
	"market_momentum_sp500": {"score": 99.0, "rating": "extreme greed", "data": []},
	"stock_price_strength": {"score": 1, "rating": "extreme fear", "data": []},
	"stock_price_breadth": {"score": 5, "rating": "extreme fear", "data": []},
	"put_call_options": {"score": 32.2, "rating": "fear", "data": []},
	"market_volatility_vix_50": {"score": 50, "rating": "neutral", "data": []},
	"market_volatility_vix": {"score": 12, "rating": "extreme fear", "data": []},
	"junk_bond_demand": {"score": 58.6, "rating": "greed", "data": []},
	"safe_haven_demand": {"score": 31, "rating": "fear", "data": []}
}`

const cryptoJSON = `{"data": [{"value": "51", "value_classification": "Neutral", "timestamp": "1789516800"},
	{"value": "69", "value_classification": "Greed", "timestamp": "1789430400"}]}`

func decode[T any](t *testing.T, text string) T {
	t.Helper()
	var out T
	if err := json.Unmarshal([]byte(text), &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func headline(rec map[string]any) [4]any {
	return [4]any{rec["index"], rec["source"], rec["score"], rec["rating"]}
}

func TestAScoreIsNamedOnThePublishersOwnScale(t *testing.T) {
	got := []string{}
	for _, v := range []float64{0, 24.9, 25, 44.9, 45, 55, 56, 75.9, 76, 100} {
		got = append(got, Band(v))
	}
	want := []string{"Extreme fear", "Extreme fear", "Fear", "Fear", "Neutral", "Neutral", "Greed", "Greed", "Extreme greed", "Extreme greed"}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("bands = %v, want %v", got, want)
	}
}

func TestThePublishersOwnWordIsKeptWhereItGivesOne(t *testing.T) {
	if got := Rating("extreme fear", 90); got != "Extreme fear" {
		t.Errorf("the publisher's word, not the scale's: %q", got)
	}
	if got := Rating("", 90); got != "Extreme greed" {
		t.Errorf("and the scale's where it gives none: %q", got)
	}
}

func TestTheReadingItsComparisonsItsSevenIndicatorsAndItsHistory(t *testing.T) {
	rec := ParseStocks(decode[Stocks](t, cnnJSON))
	if want := [4]any{"stocks", "CNN", 28.7, "Fear"}; headline(rec) != want {
		t.Errorf("reading = %v, want %v", headline(rec), want)
	}
	if rec["asOf"] != "2026-09-15T23:59:51Z" {
		t.Errorf("asOf = %v", rec["asOf"])
	}
	previous := [][3]any{}
	for _, r := range rec["previous"].([]map[string]any) {
		previous = append(previous, [3]any{r["label"], r["score"], r["rating"]})
	}
	if want := [][3]any{{"Previous close", 31.1, "Fear"}, {"A week ago", 39.1, "Fear"}, {"A month ago", 64.3, "Greed"}, {"A year ago", 64.5, "Greed"}}; !reflect.DeepEqual(previous, want) {
		t.Errorf("previous = %v, want %v", previous, want)
	}
	parts := rec["parts"].([]map[string]any)
	names := []any{}
	for _, p := range parts {
		names = append(names, p["name"])
	}
	if want := []any{"Market momentum", "Stock price strength", "Stock price breadth", "Put and call options", "Market volatility", "Junk bond demand", "Safe haven demand"}; !reflect.DeepEqual(names, want) {
		t.Errorf("parts = %v, want %v", names, want)
	}
	if parts[0]["score"] != 22.8 {
		t.Errorf("the 125-day momentum CNN's own page names: %v", parts[0]["score"])
	}
	if parts[4]["score"] != 50.0 {
		t.Errorf("and the VIX's 50-day average, not the other form in the answer: %v", parts[4]["score"])
	}
	dates := []any{}
	for _, p := range rec["series"].([]map[string]any) {
		dates = append(dates, p["date"])
	}
	if want := []any{"2025-09-16", "2026-09-15"}; !reflect.DeepEqual(dates, want) {
		t.Errorf("oldest first: %v, want %v", dates, want)
	}
}

func TestAnAnswerWithNoScoreIsNoReading(t *testing.T) {
	if got := ParseStocks(decode[Stocks](t, `{"fear_and_greed": {}}`)); len(got) != 0 {
		t.Errorf("ParseStocks(no score) = %v, want empty", got)
	}
	if got := ParseStocks(Stocks{}); len(got) != 0 {
		t.Errorf("ParseStocks(nil) = %v, want empty", got)
	}
}

func TestTheDaysOwnReadingAndTheDaysBehindIt(t *testing.T) {
	rec := ParseCrypto(decode[Crypto](t, cryptoJSON))
	if want := [4]any{"crypto", "Alternative.me", 51.0, "Neutral"}; headline(rec) != want {
		t.Errorf("reading = %v, want %v", headline(rec), want)
	}
	if rec["asOf"] != "2026-09-16T00:00:00Z" {
		t.Errorf("asOf = %v", rec["asOf"])
	}
	previous := [][2]any{}
	for _, r := range rec["previous"].([]map[string]any) {
		previous = append(previous, [2]any{r["label"], r["score"]})
	}
	if want := [][2]any{{"Yesterday", 69.0}}; !reflect.DeepEqual(previous, want) {
		t.Errorf("only the days the publisher gave: %v, want %v", previous, want)
	}
	if parts := rec["parts"].([]map[string]any); len(parts) != 0 {
		t.Errorf("it publishes no indicators under the index: %v", parts)
	}
	dates := []any{}
	for _, p := range rec["series"].([]map[string]any) {
		dates = append(dates, p["date"])
	}
	if want := []any{"2026-09-15", "2026-09-16"}; !reflect.DeepEqual(dates, want) {
		t.Errorf("series = %v, want %v", dates, want)
	}
}

func TestAnEmptyAnswerIsNoReading(t *testing.T) {
	if got := ParseCrypto(decode[Crypto](t, `{"data": []}`)); len(got) != 0 {
		t.Errorf("ParseCrypto(empty) = %v, want empty", got)
	}
}

func TestAReadingIsStoredWholeAndReadBackWhole(t *testing.T) {
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	rec := ParseStocks(decode[Stocks](t, cnnJSON))
	st.SaveGauge("stocks", rec, "", fearVersion)
	back := st.Gauge("stocks")
	if back == nil {
		t.Fatal("no gauge stored")
	}
	if back.Score == nil || *back.Score != 28.7 || back.Rating != "Fear" || back.Source != "CNN" || back.AsOf != rec["asOf"] {
		t.Errorf("back = %+v, want (28.7, Fear, CNN, %v)", back, rec["asOf"])
	}
	if parts, _ := back.Rest["parts"].([]any); len(parts) != 7 {
		t.Errorf("parts = %v, want 7", back.Rest["parts"])
	}
	got, _ := json.Marshal(back.Rest["series"])
	want, _ := json.Marshal(rec["series"])
	if string(got) != string(want) {
		t.Errorf("series = %s, want %s", got, want)
	}
	if back.ReadVersion != fearVersion {
		t.Errorf("readVersion = %d, want %d", back.ReadVersion, fearVersion)
	}
}
