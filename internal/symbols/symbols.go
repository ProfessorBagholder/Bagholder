package symbols

import (
	"regexp"
	"strings"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
)

var SpaceRE = regexp.MustCompile("[" + py.SpaceClass + `\x{200b}]+`)

var (
	putCallRE  = regexp.MustCompile(`\b(PUT|CALL)\b`)
	cpTailRE   = regexp.MustCompile(`[` + py.SpaceClass + `][CP]$`)
	occRE      = regexp.MustCompile(`^[A-Z][A-Z0-9.]{0,9} \d{6}[CP]\d+`)
	occUnderRE = regexp.MustCompile(`^([A-Z][A-Z0-9.]{0,9}) \d{6}[CP]\d+`)
	wordyUnder = regexp.MustCompile(`^([A-Z][A-Z0-9.]{0,9}) \d{1,2}[A-Z]{3}\d{2}\b`)
	compactRE  = regexp.MustCompile(`[` + py.SpaceClass + `_\-]+`)
	expiryRE   = regexp.MustCompile(`^\S+ (\d{2})([A-Z]{3})(\d{2}) `)
	listingRE  = regexp.MustCompile(`(?i)^(.+)\.(TO|V|CN|NE)$`)
	sixPutRE   = regexp.MustCompile(` \d{6}P\d+`)
	strikeRE   = regexp.MustCompile(` (\d+(?:\.\d+)?) (CALL|PUT)$`)
	Months     = []string{"Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"}
)

func Compact(s string) string {
	return compactRE.ReplaceAllString(strings.ToUpper(py.Strip(s)), "")
}

func NormAccountName(s string) string {
	return py.Strip(SpaceRE.ReplaceAllString(s, " "))
}

func folded(symbol string) string {
	return SpaceRE.ReplaceAllString(strings.ToUpper(py.Strip(symbol)), " ")
}

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

func Multiplier(symbol string) float64 {
	if IsOption(symbol) {
		return 100
	}
	return 1
}

func Right(symbol string) string {
	u := folded(symbol)
	if strings.HasSuffix(u, " PUT") || strings.HasSuffix(u, " P") || sixPutRE.MatchString(u) {
		return "PUT"
	}
	return "CALL"
}

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

func Strike(symbol string) float64 {
	m := strikeRE.FindStringSubmatch(SpaceRE.ReplaceAllString(strings.ToUpper(symbol), " "))
	if m == nil {
		return 0
	}
	return py.Num(m[1], 0)
}

func ListingTicker(sym string) string {
	s := py.Strip(sym)
	if m := listingRE.FindStringSubmatch(s); m != nil {
		return m[1]
	}
	return s
}

func TMXSymbol(symbol string) string {
	s := strings.ToUpper(py.Strip(symbol))
	for _, suffix := range []string{".TO", ".V", ".CN", ".NE"} {
		if strings.HasSuffix(s, suffix) {
			s = s[:len(s)-len(suffix)]
		}
	}
	return s
}
