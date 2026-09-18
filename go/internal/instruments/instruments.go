package instruments

import (
	"slices"
	"sort"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

var KindLabel = map[string]string{"Index": "Indices", "Future": "Futures", "Commodity": "Commodities", "Rate": "Rates", "Currency": "Currencies"}

type Instrument struct {
	Symbol   string   `json:"symbol"`
	Name     string   `json:"name"`
	Kind     string   `json:"kind"`
	Exchange string   `json:"exchange"`
	Currency string   `json:"currency"`
	Yahoo    string   `json:"yahoo"`
	Aliases  []string `json:"aliases"`
}

var Instruments = []Instrument{
	{"SPX", "S&P 500", "Index", "Index", "USD", "^GSPC", []string{"S&P", "S&P500", "SP500", "GSPC"}},
	{"NDX", "Nasdaq 100", "Index", "Index", "USD", "^NDX", []string{"NASDAQ100", "NASDAQ 100"}},
	{"IXIC", "Nasdaq Composite", "Index", "Index", "USD", "^IXIC", []string{"NASDAQ", "COMP"}},
	{"DJI", "Dow Jones Industrial Average", "Index", "Index", "USD", "^DJI", []string{"DJIA", "DOW", "DOW JONES"}},
	{"RUT", "Russell 2000", "Index", "Index", "USD", "^RUT", []string{"RUSSELL", "RUSSELL 2000"}},
	{"VIX", "CBOE Volatility Index", "Index", "Index", "USD", "^VIX", []string{"VOLATILITY"}},
	{"TSX", "S&P/TSX Composite", "Index", "Index", "CAD", "^GSPTSE", []string{"GSPTSE", "TSX COMPOSITE", "S&P/TSX"}},
	{"FTSE", "FTSE 100", "Index", "Index", "GBP", "^FTSE", []string{"FTSE 100"}},
	{"DAX", "DAX", "Index", "Index", "EUR", "^GDAXI", []string{"GDAXI"}},
	{"N225", "Nikkei 225", "Index", "Index", "JPY", "^N225", []string{"NIKKEI", "NIKKEI 225"}},
	{"HSI", "Hang Seng", "Index", "Index", "HKD", "^HSI", []string{"HANG SENG"}},
	{"STOXX50E", "Euro Stoxx 50", "Index", "Index", "EUR", "^STOXX50E", []string{"STOXX", "EURO STOXX"}},
	{"DXY", "US Dollar Index", "Index", "Index", "USD", "DX-Y.NYB", []string{"DOLLAR INDEX"}},
	{"ES", "S&P 500 E-mini futures", "Future", "CME", "USD", "ES=F", []string{"ES=F", "S&P FUTURES", "S&P 500 FUTURES", "SPX FUTURES", "ES FUTURES", "FUTURES"}},
	{"NQ", "Nasdaq 100 E-mini futures", "Future", "CME", "USD", "NQ=F", []string{"NQ=F", "NASDAQ FUTURES", "NASDAQ 100 FUTURES", "NQ FUTURES"}},
	{"YM", "Dow E-mini futures", "Future", "CBOT", "USD", "YM=F", []string{"YM=F", "DOW FUTURES", "YM FUTURES"}},
	{"RTY", "Russell 2000 E-mini futures", "Future", "CME", "USD", "RTY=F", []string{"RTY=F", "RUSSELL FUTURES", "RTY FUTURES"}},
	{"CL", "Crude Oil (WTI)", "Commodity", "NYMEX", "USD", "CL=F", []string{"WTI", "CRUDE", "OIL", "CRUDE OIL"}},
	{"BZ", "Brent Crude Oil", "Commodity", "ICE", "USD", "BZ=F", []string{"BRENT"}},
	{"NG", "Natural Gas", "Commodity", "NYMEX", "USD", "NG=F", []string{"NATGAS", "NATURAL GAS", "GAS"}},
	{"GC", "Gold", "Commodity", "COMEX", "USD", "GC=F", []string{"GOLD"}},
	{"SI", "Silver", "Commodity", "COMEX", "USD", "SI=F", []string{"SILVER"}},
	{"HG", "Copper", "Commodity", "COMEX", "USD", "HG=F", []string{"COPPER"}},
	{"PL", "Platinum", "Commodity", "NYMEX", "USD", "PL=F", []string{"PLATINUM"}},
	{"ZC", "Corn", "Commodity", "CBOT", "USD", "ZC=F", []string{"CORN"}},
	{"ZW", "Wheat", "Commodity", "CBOT", "USD", "ZW=F", []string{"WHEAT"}},
	{"TNX", "US 10-Year Treasury Yield", "Rate", "Index", "USD", "^TNX", []string{"10Y", "10-YEAR", "10 YEAR", "TREASURY", "YIELD"}},
	{"ZQ", "30-Day Federal Funds futures", "Rate", "CBOT", "USD", "ZQ=F", []string{"FED", "FED FUNDS", "FED FUNDS FUTURES", "FEDERAL FUNDS", "FOMC", "POLICY RATE", "ZQ=F"}},
	{"SR3", "Three-Month SOFR futures", "Rate", "CME", "USD", "SR3=F", []string{"SOFR", "SOFR FUTURES", "THREE-MONTH SOFR", "SR3=F"}},
	{"USDCAD", "US Dollar / Canadian Dollar", "Currency", "FX", "CAD", "CAD=X", []string{"USD/CAD", "CAD", "LOONIE"}},
	{"EURUSD", "Euro / US Dollar", "Currency", "FX", "USD", "EURUSD=X", []string{"EUR/USD", "EURO"}},
	{"GBPUSD", "British Pound / US Dollar", "Currency", "FX", "USD", "GBPUSD=X", []string{"GBP/USD", "POUND"}},
	{"USDJPY", "US Dollar / Japanese Yen", "Currency", "FX", "JPY", "JPY=X", []string{"USD/JPY", "YEN"}},
	{"BTCUSD", "Bitcoin / US Dollar", "Currency", "FX", "USD", "BTC-USD", []string{"BITCOIN", "BTC"}},
}

var Labels = map[string]string{"ZQ": "FED FUNDS", "SR3": "SOFR", "CL": "WTI", "BZ": "BRENT", "NG": "NATGAS", "GC": "GOLD", "SI": "SILVER", "HG": "COPPER", "PL": "PLATINUM", "ZC": "CORN", "ZW": "WHEAT",
	"TNX": "10Y", "USDCAD": "USD/CAD", "EURUSD": "EUR/USD", "GBPUSD": "GBP/USD", "USDJPY": "USD/JPY", "BTCUSD": "BITCOIN"}

var RateFromPrice = map[string]string{"ZQ": "Implied rate", "SR3": "Implied rate"}

func ImpliedRate(symbol string, price *float64) *float64 {
	if _, ok := RateFromPrice[strings.ToUpper(strings.TrimSpace(symbol))]; !ok || price == nil {
		return nil
	}
	return py.Ptr(py.Round(100.0-*price, 4))
}

func Label(symbol string) string {
	sym := strings.ToUpper(strings.TrimSpace(symbol))
	if l, ok := Labels[sym]; ok {
		return l
	}
	return sym
}

func Rows() []Instrument {
	out := make([]Instrument, len(Instruments))
	for i, r := range Instruments {
		out[i] = r
		out[i].Aliases = append([]string{}, r.Aliases...)
	}
	return out
}

func Find(symbol, exchange string) *Instrument {
	sym, ex := strings.ToUpper(strings.TrimSpace(symbol)), strings.ToUpper(strings.TrimSpace(exchange))
	for i := range Instruments {
		r := Instruments[i]
		if r.Symbol == sym && strings.ToUpper(r.Exchange) == ex {
			out := r
			out.Aliases = append([]string{}, r.Aliases...)
			return &out
		}
	}
	return nil
}

type Match struct {
	Symbol   string `json:"symbol"`
	Name     string `json:"name"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
	Kind     string `json:"kind"`
	Rank     int    `json:"rank"`
}

func Search(text string) []Match {
	q := strings.ToUpper(strings.TrimSpace(text))
	if q == "" {
		return []Match{}
	}
	type ranked struct {
		rank int
		m    Match
	}
	var out []ranked
	for _, r := range Instruments {
		names := []string{r.Symbol}
		for _, a := range r.Aliases {
			names = append(names, strings.ToUpper(a))
		}
		var words []string
		for _, x := range append(append([]string{}, names...), strings.ToUpper(r.Name)) {
			words = append(words, py.Fields(strings.ReplaceAll(x, "/", " "))...)
		}
		rank := -1
		if slices.Contains(names, q) {
			rank = 0
		} else if len([]rune(q)) < 2 {
			rank = -1
		} else if startsAny(append(append([]string{}, names...), strings.ToUpper(r.Name)), q) {
			rank = 1
		} else if startsAny(words, q) {
			rank = 2
		}
		if rank >= 0 {
			out = append(out, ranked{rank, Match{Symbol: r.Symbol, Name: r.Name, Exchange: r.Exchange, Currency: r.Currency, Kind: r.Kind, Rank: rank}})
		}
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].rank < out[j].rank })
	res := make([]Match, len(out))
	for i, r := range out {
		res[i] = r.m
	}
	return res
}

func startsAny(list []string, q string) bool {
	for _, x := range list {
		if strings.HasPrefix(x, q) {
			return true
		}
	}
	return false
}
