package universes

import (
	"encoding/json"
	"reflect"
	"testing"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const screenerJSON = `{"data": {"rows": [
	{"symbol": "NVDA", "name": "NVIDIA Corporation Common Stock", "lastsale": "$219.41", "pctchange": "0.808%", "marketCap": "5350000000000.00", "sector": "Technology", "country": "United States"},
	{"symbol": "JPM", "name": "JP Morgan", "lastsale": "$210.00", "pctchange": "-0.5%", "marketCap": "600000000000.00", "sector": "Finance", "country": "United States"},
	{"symbol": "TSM", "name": "Taiwan Semiconductor", "lastsale": "$250.00", "pctchange": "1.2%", "marketCap": "1300000000000.00", "sector": "Technology", "country": "Taiwan"},
	{"symbol": "SHOP", "name": "Shopify", "lastsale": "$130.00", "pctchange": "3.3%", "marketCap": "170000000000.00", "sector": "Technology", "country": "Canada"},
	{"symbol": "XYZ", "name": "No cap", "lastsale": "$1.00", "pctchange": "N/A", "marketCap": "", "sector": "Miscellaneous", "country": "United States"},
	{"symbol": "ABC", "name": "No country", "lastsale": "$2.00", "pctchange": "0.1%", "marketCap": "100.00", "sector": "Telecommunications", "country": ""}
]}}`

func decode[T any](t *testing.T, text string) T {
	t.Helper()
	var out T
	if err := json.Unmarshal([]byte(text), &out); err != nil {
		t.Fatal(err)
	}
	return out
}

func symbols(rows []store.Universe) []string {
	out := []string{}
	for _, r := range rows {
		out = append(out, r.Symbol)
	}
	return out
}

func TestRowsAreParsedAndSectorsFolded(t *testing.T) {
	rows := ParseScreener(decode[Screener](t, screenerJSON))
	type view struct {
		Symbol        string
		Last          *float64
		PercentChange *float64
		Cap           float64
		Sector        string
		Country       string
	}
	got := []view{}
	for _, r := range rows[:2] {
		got = append(got, view{r.Symbol, r.Last, r.PercentChange, r.Cap, r.Sector, r.Country})
	}
	want := []view{
		{"NVDA", py.Ptr(219.41), py.Ptr(0.808), 5.35e12, "Information Technology", "United States"},
		{"JPM", py.Ptr(210.0), py.Ptr(-0.5), 6e11, "Financials", "United States"},
	}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("rows[:2] = %+v, want %+v", got, want)
	}
	sectors := []string{}
	for _, r := range rows[4:] {
		sectors = append(sectors, r.Sector)
	}
	if want := []string{"Not classified", "Communication Services"}; !reflect.DeepEqual(sectors, want) {
		t.Errorf("sectors[4:] = %v, want %v", sectors, want)
	}
	if rows[4].PercentChange != nil {
		t.Errorf("rows[4].PercentChange = %v, want nil", *rows[4].PercentChange)
	}
}

func TestUSAndInternationalAreTheLargestByCap(t *testing.T) {
	rows := ParseScreener(decode[Screener](t, screenerJSON))
	if got := symbols(USRows(rows, 5)); !reflect.DeepEqual(got, []string{"NVDA", "JPM"}) {
		t.Errorf("US companies with a market cap, largest first: %v", got)
	}
	if got := symbols(IntlRows(rows, 5)); !reflect.DeepEqual(got, []string{"TSM"}) {
		t.Errorf("foreign companies listed in the US; Canada and blanks excluded: %v", got)
	}
	if v := USRows(rows, 1)[0].Value; v == nil || *v != 5.35e12 {
		t.Errorf("sized by market cap: %v", py.Deref(v, -1))
	}
}

func TestConstituentsAndTileQuote(t *testing.T) {
	cons := ParseConstituents(decode[Constituents](t, `{"data": {"constituents": [{"symbol": "RY", "quotedMarketValue": 398317400940, "longName": "Royal Bank of Canada", "weight": 9.823, "exchange": "TSX"}, {"weight": 1}]}}`))
	if want := []Constituent{{Symbol: "RY", Name: "Royal Bank of Canada", Weight: 9.823, Cap: 398317400940.0, Exchange: "TSX"}}; !reflect.DeepEqual(cons, want) {
		t.Errorf("cons = %+v, want %+v", cons, want)
	}
	q := ParseTileQuote(decode[Tile](t, `{"data": {"getQuoteBySymbol": {"symbol": "RY", "name": "Royal Bank", "price": 180.1, "percentChange": 0.42, "sector": "Financial Services"}}}`))
	if want := (&TileQuote{PercentChange: py.Ptr(0.42), Sector: "Financials", Name: "Royal Bank"}); !reflect.DeepEqual(q, want) {
		t.Errorf("q = %+v, want %+v", q, want)
	}
	if got := ParseTileQuote(decode[Tile](t, `{"data": {"getQuoteBySymbol": null}}`)); got != nil {
		t.Errorf("q = %+v, want nil", got)
	}
}

func TestReplaceAndSnapshot(t *testing.T) {
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	before := st.DataVersion()
	st.ReplaceUniverse("ca", []store.Universe{{Symbol: "RY", Name: "Royal Bank", Value: py.Ptr(9.823), PercentChange: py.Ptr(0.42), Sector: "Financials", Country: "Canada"}}, "2026-09-11T16:00:00Z")
	st.ReplaceUniverse("ca", []store.Universe{{Symbol: "TD", Name: "TD", Value: py.Ptr(6.9), Sector: "Financials", Country: "Canada"}}, "2026-09-11T16:30:00Z")
	u := st.Snapshot(false).Universes
	type view struct {
		Symbol        string
		Value         *float64
		PercentChange *float64
		FetchedAt     string
	}
	got := []view{}
	for _, r := range u["ca"] {
		got = append(got, view{r.Symbol, r.Value, r.PercentChange, r.FetchedAt})
	}
	if want := []view{{"TD", py.Ptr(6.9), nil, "2026-09-11T16:30:00Z"}}; !reflect.DeepEqual(got, want) {
		t.Errorf("an answer replaces the universe's rows: %+v, want %+v", got, want)
	}
	if _, ok := u["us"]; ok {
		t.Errorf("us in universes: %v", u["us"])
	}
	if st.DataVersion() == before {
		t.Errorf("data version unchanged: %s", before)
	}
}
