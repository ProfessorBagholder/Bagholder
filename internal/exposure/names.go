// Package exposure classifies the book's holdings by sector and country: a share by
// its listing's own record, a fund looked through to its holdings from the issuer's
// published record. Nothing here asks Wealthsimple for anything.
package exposure

import (
	"regexp"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

const (
	ShareKey = "share:"
	FundKey  = "fund:"
)

// Sectors is the one set the Portfolio folds every source's names onto.
var Sectors = []string{"Information Technology", "Financials", "Health Care", "Consumer Discretionary", "Consumer Staples", "Industrials", "Energy", "Materials", "Utilities", "Real Estate", "Communication Services"}

var sectorAlias = map[string]string{
	"technology": "Information Technology", "information technology": "Information Technology", "tech": "Information Technology",
	"financial": "Financials", "financials": "Financials", "financial services": "Financials", "finance": "Financials", "banks": "Financials",
	"health care": "Health Care", "healthcare": "Health Care",
	"consumer discretionary": "Consumer Discretionary", "consumer cyclicals": "Consumer Discretionary", "consumer, cyclical": "Consumer Discretionary", "consumer cyclical": "Consumer Discretionary",
	"consumer staples": "Consumer Staples", "consumer non-cyclicals": "Consumer Staples", "consumer, non-cyclical": "Consumer Staples", "consumer non-cyclical": "Consumer Staples", "consumer defensive": "Consumer Staples",
	"industrials": "Industrials", "industrial": "Industrials",
	"energy":    "Energy",
	"materials": "Materials", "basic materials": "Materials",
	"utilities":   "Utilities",
	"real estate": "Real Estate", "realestate": "Real Estate",
	"communication services": "Communication Services", "communications": "Communication Services", "communication": "Communication Services", "media": "Communication Services", "telecommunications services": "Communication Services", "telecommunications": "Communication Services", "telecommunication services": "Communication Services",
	"bitcoin holding": "Digital assets", "digital assets": "Digital assets", "cryptocurrency": "Digital assets", "crypto": "Digital assets",
	"cash and/or derivatives": "", "cash": "", "other": "", "miscellaneous": "", "-": "", "n/a": "",
}

// NormSector is the sector under the name the Portfolio uses, "" for none (cash, other, blank).
func NormSector(name string) string {
	key := strings.ToLower(py.Strip(name))
	if key == "" {
		return ""
	}
	if v, ok := sectorAlias[key]; ok {
		return v
	}
	return py.Strip(name)
}

var countryAlias = map[string]string{
	"united states": "United States", "united states of america": "United States", "usa": "United States", "us": "United States", "u.s.": "United States", "u.s.a.": "United States",
	"canada": "Canada", "ca": "Canada", "can": "Canada",
	"united kingdom": "United Kingdom", "uk": "United Kingdom", "gb": "United Kingdom", "great britain": "United Kingdom", "britain": "United Kingdom",
	"korea": "South Korea", "korea, republic of": "South Korea", "republic of korea": "South Korea", "south korea": "South Korea",
	"taiwan, province of china": "Taiwan", "taiwan": "Taiwan", "hong kong sar": "Hong Kong", "hong kong": "Hong Kong",
	"russian federation": "Russia", "viet nam": "Vietnam", "czech republic": "Czechia",
	"broad": "", "global": "", "other": "", "-": "", "n/a": "", "cash": "",
}

// VenueCountry says which country a listing venue is in.
var VenueCountry = map[string]string{
	"TSX": "Canada", "TSX-V": "Canada", "TSXV": "Canada", "CSE": "Canada", "CBOE CANADA": "Canada", "NEO": "Canada", "ALPHA EXCHANGE": "Canada",
	"TORONTO STOCK EXCHANGE": "Canada", "TSX VENTURE EXCHANGE": "Canada", "CANADIAN SECURITIES EXCHANGE": "Canada",
	"NYSE": "United States", "NASDAQ": "United States", "NYSE ARCA": "United States", "NYSE AMERICAN": "United States", "BATS": "United States", "AMEX": "United States", "ARCA": "United States",
	"NASDAQ GLOBAL SELECT": "United States", "NASDAQ GLOBAL MARKET": "United States", "NASDAQ CAPITAL MARKET": "United States", "NEW YORK STOCK EXCHANGE": "United States",
}

// BloombergCountry is Bloomberg's market codes, as issuers write tickers ("MSFT US EQUITY").
var BloombergCountry = map[string]string{"US": "United States", "UN": "United States", "UW": "United States", "UQ": "United States", "UA": "United States", "CN": "Canada", "CT": "Canada", "CV": "Canada",
	"LN": "United Kingdom", "JP": "Japan", "JT": "Japan", "GR": "Germany", "GY": "Germany", "FP": "France", "AU": "Australia", "AT": "Australia", "HK": "Hong Kong",
	"SW": "Switzerland", "SE": "Switzerland", "NA": "Netherlands", "SM": "Spain", "IM": "Italy", "KS": "South Korea", "TT": "Taiwan", "IN": "India", "IS": "India",
	"BZ": "Brazil", "SS": "Sweden", "DC": "Denmark", "NO": "Norway", "FH": "Finland", "BB": "Belgium", "ID": "Ireland", "SP": "Singapore", "MM": "Mexico", "CH": "China", "C1": "China"}

// NormCountry is a country under the app's own name for it.
func NormCountry(name string) string {
	key := strings.ToLower(py.Strip(name))
	if key == "" {
		return ""
	}
	if v, ok := countryAlias[key]; ok {
		return v
	}
	return py.Strip(name)
}

// VenueCountryOf is the country a listing venue is in, "" when unknown.
func VenueCountryOf(exchange string) string {
	return VenueCountry[strings.ToUpper(py.Strip(exchange))]
}

var issuers = []struct {
	key string
	re  *regexp.Regexp
}{
	{"vanguard", regexp.MustCompile(`^vanguard\b`)}, {"ishares", regexp.MustCompile(`^ishares\b`)}, {"harvest", regexp.MustCompile(`^harvest\b`)}, {"ninepoint", regexp.MustCompile(`^ninepoint\b`)}, {"evolve", regexp.MustCompile(`^evolve\b`)},
	{"bmo", regexp.MustCompile(`^bmo\b`)}, {"globalx", regexp.MustCompile(`^(global x|horizons)\b`)},
}

// IssuerOf is the fund family a name belongs to, "" for none.
func IssuerOf(name string) string {
	n := strings.ToLower(py.Strip(name))
	for _, i := range issuers {
		if i.re.MatchString(n) {
			return i.key
		}
	}
	return ""
}

var fundRE = regexp.MustCompile(`(?i)\b(ETF|Index|Fund|Portfolio|Trust)\b`)

// IsFund is whether a name reads as a fund.
func IsFund(name string) bool {
	return fundRE.MatchString(name) || IssuerOf(name) != ""
}
