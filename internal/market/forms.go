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

// YahooVenues is what each Yahoo suffix names.
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

// YahooSplit is `YES.V` as Yahoo writes it: the bare ticker and the venues the suffix names; (text, nil) without one.
func YahooSplit(text string) (string, []string) {
	s := strings.ToUpper(strings.TrimSpace(text))
	for _, yv := range YahooVenues {
		if strings.HasSuffix(s, yv.Suffix) && len(s) > len(yv.Suffix) {
			return s[:len(s)-len(yv.Suffix)], yv.Venues
		}
	}
	return s, nil
}

// TMXSymbol is the bare ticker TMX Money names a listing by.
func TMXSymbol(symbol string) string { return symbols.TMXSymbol(symbol) }

// TMXForm is TMX's symbol suffix for a listing venue, or nil when TMX does not carry it.
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

// TMXFormOr is TMXForm with "" for none, as `tmx_form(...) or ""` read.
func TMXFormOr(exchange, currency string) string {
	if f := TMXForm(exchange, currency); f != nil {
		return *f
	}
	return ""
}

// TMXRecordSymbol is the TMX Money symbol for a Canadian listing's declared distribution record, "" when none.
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

// TMXQuoteSymbol is the TMX Money symbol for a listing in the form its venue takes; "" when TMX does not carry it.
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

// TMXBare is the part of a TMX key before its venue form.
func TMXBare(key string) string {
	k, _, _ := strings.Cut(key, ":")
	return k
}

// IsCanadianListing is whether a listing is on a Canadian venue, by its venue else its currency.
func IsCanadianListing(exchange, currency string) bool {
	ex := strings.ToUpper(strings.TrimSpace(exchange))
	if ex != "" {
		return in(CanadianExchanges, ex)
	}
	return strings.ToUpper(strings.TrimSpace(currency)) == "CAD"
}

// YahooRoot is the ticker as Yahoo writes it: dots to dashes.
func YahooRoot(symbol string) string {
	return strings.ReplaceAll(TMXSymbol(symbol), ".", "-")
}

// Rec is what a source needs to price or chart an instrument.
type Rec struct {
	Symbol   string `json:"symbol"`
	Exchange string `json:"exchange"`
	Currency string `json:"currency"`
	Kind     string `json:"kind"`
	QuoteKey string `json:"quoteKey,omitempty"`
	Yahoo    string `json:"yahoo,omitempty"`
	Start    string `json:"start,omitempty"`
}

// YahooFormsFor is the Yahoo symbols for a share listing, the venue's own suffix first.
func YahooFormsFor(rec Rec) []string {
	root := YahooRoot(rec.Symbol)
	ccy := strings.ToUpper(strings.TrimSpace(rec.Currency))
	if ccy == "" {
		ccy = "CAD"
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
