package market

import (
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/symbols"
)

var USExchanges = []string{"NASDAQ", "NYSE", "NYSE AMERICAN", "NYSE ARCA", "BATS", "AMEX", "ARCA", "CBOE", "IEX"}
var CboeCanadaExchanges = []string{"CBOE CANADA", "NEO"}
var CanadianExchanges = []string{"TSX", "TSX-V", "TSXV", "CSE", "CBOE CANADA", "NEO", "ALPHA EXCHANGE"}

var YahooSuffix = map[string]string{"TSX": ".TO", "TSX-V": ".V", "TSXV": ".V", "CSE": ".CN", "CBOE CANADA": ".NE", "NEO": ".NE"}
var YahooForms = map[string][]string{"CAD": {".TO", ".V", ".CN", ".NE"}, "USD": {""}}

var YahooVenues = []struct {
	Suffix string
	Venues []string
}{{".TO", []string{"TSX"}}, {".V", []string{"TSX-V", "TSXV"}}, {".CN", []string{"CSE"}}, {".NE", []string{"CBOE CANADA", "NEO"}}}

func in(list []string, s string) bool {
	for _, x := range list {
		if x == s {
			return true
		}
	}
	return false
}

func YahooSplit(text string) (string, []string) {
	s := strings.ToUpper(strings.TrimSpace(text))
	for _, yv := range YahooVenues {
		if strings.HasSuffix(s, yv.Suffix) && len(s) > len(yv.Suffix) {
			return s[:len(s)-len(yv.Suffix)], yv.Venues
		}
	}
	return s, nil
}

func TMXSymbol(symbol string) string { return symbols.TMXSymbol(symbol) }

func TMXForm(exchange, currency string) *string {
	ex := strings.ToUpper(strings.TrimSpace(exchange))
	ccy := strings.ToUpper(strings.TrimSpace(currency))
	form := func(s string) *string { return &s }
	if in(USExchanges, ex) || (ex == "" && ccy == "USD") {
		return form(":US")
	}
	if in(CboeCanadaExchanges, ex) {
		return form(":AQL")
	}
	if ex == "CSE" {
		return form(":CNX")
	}
	if ex == "TSX" || ex == "TSX-V" || ex == "TSXV" {
		return form("")
	}
	if ccy == "CAD" {
		return form("")
	}
	if ccy == "USD" {
		return form(":US")
	}
	return nil
}

func TMXFormOr(exchange, currency string) string {
	if f := TMXForm(exchange, currency); f != nil {
		return *f
	}
	return ""
}

func TMXRecordSymbol(symbol, exchange string) string {
	s := TMXSymbol(symbol)
	if s == "" {
		return ""
	}
	form := TMXForm(exchange, "CAD")
	if form == nil {
		return ""
	}
	return s + *form
}

func TMXQuoteSymbol(symbol, exchange, currency string) string {
	s := TMXSymbol(symbol)
	if s == "" || strings.Contains(s, " ") {
		return ""
	}
	form := TMXForm(exchange, currency)
	if form == nil {
		return ""
	}
	return s + *form
}

func TMXBare(key string) string {
	k, _, _ := strings.Cut(key, ":")
	return k
}

func IsCanadianListing(exchange, currency string) bool {
	ex := strings.ToUpper(strings.TrimSpace(exchange))
	if ex != "" {
		return in(CanadianExchanges, ex)
	}
	return strings.ToUpper(strings.TrimSpace(currency)) == "CAD"
}

func YahooRoot(symbol string) string {
	return strings.ReplaceAll(TMXSymbol(symbol), ".", "-")
}

type Rec struct {
	Symbol   string `json:"symbol"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
	Kind     string `json:"kind"`
	QuoteKey string `json:"quoteKey,omitempty"`
	Yahoo    string `json:"yahoo,omitempty"`
	Start    string `json:"start,omitempty"`
}

func YahooFormsFor(rec Rec) []string {
	root := YahooRoot(rec.Symbol)
	ccy := strings.ToUpper(strings.TrimSpace(rec.Currency))
	if ccy == "" {
		ccy = "CAD"
	}
	if venue := TMXForm(rec.Exchange, ""); venue != nil {
		ccy = "CAD"
		if *venue == ":US" {
			ccy = "USD"
		}
	}
	forms, ok := YahooForms[ccy]
	if root == "" || strings.Contains(root, " ") || !ok {
		return []string{}
	}
	first, has := YahooSuffix[strings.ToUpper(strings.TrimSpace(rec.Exchange))]
	list := append([]string{}, forms...)
	if has && in(list, first) {
		rest := []string{}
		for _, f := range list {
			if f != first {
				rest = append(rest, f)
			}
		}
		list = append([]string{first}, rest...)
	}
	out := make([]string, len(list))
	for i, f := range list {
		out[i] = root + f
	}
	return out
}
