// Package symbols reads instrument symbols the way every part of the app must
// agree on: whether one is an option contract, its underlying, its multiplier,
// its right and its expiry, and the bare ticker a listing goes by.
package symbols

import (
	"regexp"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

// SpaceRE is every whitespace the model folds, Python's \s and the typographic spaces Wealthsimple writes.
var SpaceRE = regexp.MustCompile("[" + py.SpaceClass + `\x{200b}]+`)

var (
	putCallRE   = regexp.MustCompile(`\b(PUT|CALL)\b`)
	cpTailRE    = regexp.MustCompile(`[` + py.SpaceClass + `][CP]$`)
	occRE       = regexp.MustCompile(`^[A-Z][A-Z0-9.]{0,9} \d{6}[CP]\d+`)
	occUnderRE  = regexp.MustCompile(`^([A-Z][A-Z0-9.]{0,9}) \d{6}[CP]\d+`)
	wordyUnder  = regexp.MustCompile(`^([A-Z][A-Z0-9.]{0,9}) \d{1,2}[A-Z]{3}\d{2}\b`)
	compactRE   = regexp.MustCompile(`[` + py.SpaceClass + `_\-]+`)
	expiryRE    = regexp.MustCompile(`^\S+ (\d{2})([A-Z]{3})(\d{2}) `)
	listingRE   = regexp.MustCompile(`(?i)^(.+)\.(TO|V|CN|NE)$`)
	sixPutRE    = regexp.MustCompile(` \d{6}P\d+`)
	strikeRE    = regexp.MustCompile(` (\d+(?:\.\d+)?) (CALL|PUT)$`)
	Months      = []string{"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"}
)

// Compact is a string upper-cased with every space, underscore and dash removed.
func Compact(s string) string {
	return compactRE.ReplaceAllString(strings.ToUpper(py.Strip(s)), "")
}

// NormAccountName folds the ASCII and en-space separators nicknames mix.
func NormAccountName(s string) string {
	return py.Strip(SpaceRE.ReplaceAllString(s, " "))
}

func folded(symbol string) string {
	return SpaceRE.ReplaceAllString(strings.ToUpper(py.Strip(symbol)), " ")
}

// IsOption is whether a symbol names an option contract.
func IsOption(symbol string) bool {
	u := folded(symbol)
	if u == "" {
		return false
	}
	if putCallRE.MatchString(u) || cpTailRE.MatchString(u) {
		return true
	}
	return occRE.MatchString(u)
}

// Underlying is the contract's underlying ticker, the symbol itself for a listing, "—" for nothing.
func Underlying(symbol string) string {
	s := py.Strip(symbol)
	if s == "" {
		return "—"
	}
	u := SpaceRE.ReplaceAllString(strings.ToUpper(s), " ")
	if putCallRE.MatchString(u) || cpTailRE.MatchString(u) {
		first, _, _ := strings.Cut(u, " ")
		if first == "" {
			return s
		}
		return first
	}
	if m := occUnderRE.FindStringSubmatch(u); m != nil {
		return m[1]
	}
	if m := wordyUnder.FindStringSubmatch(u); m != nil {
		return m[1]
	}
	return s
}

// Multiplier is 100 for a contract, 1 otherwise.
func Multiplier(symbol string) float64 {
	if IsOption(symbol) {
		return 100
	}
	return 1
}

// Right is PUT or CALL.
func Right(symbol string) string {
	u := folded(symbol)
	if strings.HasSuffix(u, " PUT") || strings.HasSuffix(u, " P") || sixPutRE.MatchString(u) {
		return "PUT"
	}
	return "CALL"
}

// Expiry is 'LUNR 29AUG25 11.50 CALL' -> '2025-08-29', "" when the symbol carries none.
func Expiry(symbol string) string {
	m := expiryRE.FindStringSubmatch(folded(symbol))
	if m == nil {
		return ""
	}
	mon := py.Title(m[2])
	for i, name := range Months {
		if name == mon {
			return "20" + m[3] + "-" + pad2(i+1) + "-" + m[1]
		}
	}
	return ""
}

func pad2(n int) string {
	if n < 10 {
		return "0" + string(rune('0'+n))
	}
	return string(rune('0'+n/10)) + string(rune('0'+n%10))
}

// Strike is the strike a wordy contract name carries, 0 when none.
func Strike(symbol string) float64 {
	m := strikeRE.FindStringSubmatch(SpaceRE.ReplaceAllString(strings.ToUpper(symbol), " "))
	if m == nil {
		return 0
	}
	return py.Num(m[1], 0)
}

// ListingTicker drops a listing's .TO, .V, .CN or .NE suffix.
func ListingTicker(sym string) string {
	s := py.Strip(sym)
	if m := listingRE.FindStringSubmatch(s); m != nil {
		return m[1]
	}
	return s
}

// TMXSymbol is the bare ticker as TMX Money names it: upper-cased, every venue suffix dropped.
func TMXSymbol(symbol string) string {
	s := strings.ToUpper(py.Strip(symbol))
	for _, suffix := range []string{".TO", ".V", ".CN", ".NE"} {
		if strings.HasSuffix(s, suffix) {
			s = s[:len(s)-len(suffix)]
		}
	}
	return s
}
