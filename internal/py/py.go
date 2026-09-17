package py

import (
	"crypto/rand"
	"fmt"
	"math"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"
)

var reCache sync.Map

func RE(pattern string) *regexp.Regexp {
	if v, ok := reCache.Load(pattern); ok {
		return v.(*regexp.Regexp)
	}
	re := regexp.MustCompile(pattern)
	reCache.Store(pattern, re)
	return re
}

func S(v any) string {
	switch x := v.(type) {
	case nil:
		return ""
	case string:
		return x
	case bool:
		if x {
			return "True"
		}
		return "False"
	case float64:
		return Repr(x)
	case float32:
		return Repr(float64(x))
	case int:
		return strconv.Itoa(x)
	case int64:
		return strconv.FormatInt(x, 10)
	case *string:
		if x == nil {
			return ""
		}
		return *x
	case *float64:
		if x == nil {
			return ""
		}
		return Repr(*x)
	}
	return fmt.Sprint(v)
}

func JSONStr(v any) string {
	if f, ok := v.(float64); ok && !math.IsNaN(f) && !math.IsInf(f, 0) && f == math.Trunc(f) {
		return strconv.FormatFloat(f, 'f', -1, 64)
	}
	return S(v)
}

func Repr(f float64) string {
	if math.IsNaN(f) {
		return "nan"
	}
	if math.IsInf(f, 1) {
		return "inf"
	}
	if math.IsInf(f, -1) {
		return "-inf"
	}
	if f == 0 {
		if math.Signbit(f) {
			return "-0.0"
		}
		return "0.0"
	}
	e := strconv.FormatFloat(f, 'e', -1, 64)
	mant, expS, _ := strings.Cut(e, "e")
	exp, _ := strconv.Atoi(expS)
	neg := strings.HasPrefix(mant, "-")
	mant = strings.TrimPrefix(mant, "-")
	digits := strings.Replace(mant, ".", "", 1)
	var out string
	if exp >= -4 && exp < 16 {
		if exp >= 0 {
			if len(digits) <= exp+1 {
				out = digits + strings.Repeat("0", exp+1-len(digits)) + ".0"
			} else {
				out = digits[:exp+1] + "." + digits[exp+1:]
			}
		} else {
			out = "0." + strings.Repeat("0", -exp-1) + digits
		}
	} else {
		m := digits[:1]
		if len(digits) > 1 {
			m += "." + digits[1:]
		}
		sign := "+"
		if exp < 0 {
			sign = "-"
			exp = -exp
		}
		out = fmt.Sprintf("%se%s%02d", m, sign, exp)
	}
	if neg {
		return "-" + out
	}
	return out
}

func Num(v any, def float64) float64 {
	f, ok := NumOK(v)
	if !ok {
		return def
	}
	return f
}

func NumOK(v any) (float64, bool) {
	switch x := v.(type) {
	case nil:
		return 0, false
	case float64:
		return x, true
	case float32:
		return float64(x), true
	case int:
		return float64(x), true
	case int64:
		return float64(x), true
	case bool:
		if x {
			return 1, true
		}
		return 0, true
	case string:
		s := strings.TrimSpace(x)
		if s == "" {
			return 0, false
		}
		f, err := strconv.ParseFloat(s, 64)
		if err != nil {
			return 0, false
		}
		return f, true
	case *float64:
		if x == nil {
			return 0, false
		}
		return *x, true
	}
	return 0, false
}

func Ptr(f float64) *float64 { return &f }

func Deref(p *float64, def float64) float64 {
	if p == nil {
		return def
	}
	return *p
}

func Round(x float64, n int) float64 {
	if math.IsNaN(x) || math.IsInf(x, 0) {
		return x
	}
	if n <= 0 {
		p := math.Pow(10, float64(-n))
		return math.RoundToEven(x/p) * p
	}
	s := strconv.FormatFloat(x, 'f', n, 64)
	f, err := strconv.ParseFloat(s, 64)
	if err != nil {
		return x
	}
	return f
}

func RoundInt(x float64) int {
	return int(math.RoundToEven(x))
}

func G(x float64) string {
	return strconv.FormatFloat(x, 'g', 6, 64)
}

func Fixed(x float64, n int) string {
	return strconv.FormatFloat(x, 'f', n, 64)
}

func IsInteger(x float64) bool {
	return !math.IsInf(x, 0) && !math.IsNaN(x) && x == math.Trunc(x)
}

func Commas(n float64) string {
	s := strconv.FormatFloat(math.Abs(n), 'f', 0, 64)
	var b strings.Builder
	if n < 0 {
		b.WriteByte('-')
	}
	for i, c := range s {
		if i > 0 && (len(s)-i)%3 == 0 {
			b.WriteByte(',')
		}
		b.WriteRune(c)
	}
	return b.String()
}

func IsSpace(r rune) bool {
	switch r {
	case '\t', '\n', '\v', '\f', '\r', ' ', 0x1c, 0x1d, 0x1e, 0x1f, 0x85, 0xa0, 0x1680, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000:
		return true
	}
	return r >= 0x2000 && r <= 0x200a
}

const SpaceClass = `\t\n\x0b\x0c\r\x1c-\x1f \x{85}\x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}`

func Strip(s string) string {
	return strings.TrimFunc(s, IsSpace)
}

func CollapseSpace(s string) string {
	var b strings.Builder
	space := false
	for _, r := range s {
		if IsSpace(r) {
			space = true
			continue
		}
		if space {
			b.WriteByte(' ')
			space = false
		}
		b.WriteRune(r)
	}
	if space {
		b.WriteByte(' ')
	}
	return b.String()
}

func Fields(s string) []string {
	return strings.FieldsFunc(s, IsSpace)
}

func Title(s string) string {
	var b strings.Builder
	prev := false
	for _, r := range s {
		if unicode.IsLetter(r) {
			if prev {
				b.WriteRune(unicode.ToLower(r))
			} else {
				b.WriteRune(unicode.ToUpper(r))
			}
			prev = true
		} else {
			b.WriteRune(r)
			prev = false
		}
	}
	return b.String()
}

func Capitalize(s string) string {
	if s == "" {
		return s
	}
	r := []rune(s)
	return string(unicode.ToUpper(r[0])) + string(r[1:])
}

func IsWordRune(r rune) bool {
	return r == '_' || unicode.IsLetter(r) || unicode.IsDigit(r) || unicode.IsNumber(r)
}

func Lines(s string) []string {
	var out []string
	start := 0
	rs := []rune(s)
	for i := 0; i < len(rs); i++ {
		r := rs[i]
		switch r {
		case '\n', '\r', '\v', '\f', 0x1c, 0x1d, 0x1e, 0x85, 0x2028, 0x2029:
			out = append(out, string(rs[start:i]))
			if r == '\r' && i+1 < len(rs) && rs[i+1] == '\n' {
				i++
			}
			start = i + 1
		}
	}
	if start < len(rs) {
		out = append(out, string(rs[start:]))
	}
	return out
}

func UUID4() string {
	var b [16]byte
	if _, err := rand.Read(b[:]); err != nil {
		panic(err)
	}
	b[6] = (b[6] & 0x0f) | 0x40
	b[8] = (b[8] & 0x3f) | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

func Stamp(t time.Time) string {
	return t.UTC().Format("2006-01-02T15:04:05Z")
}

func NowStamp() string {
	return Stamp(time.Now())
}

func ParseISO(s string) (t time.Time, naive bool, ok bool) {
	s = strings.TrimSpace(s)
	if len(s) < 10 {
		return time.Time{}, false, false
	}
	if len(s) == 10 {
		t, err := time.ParseInLocation("2006-01-02", s, time.UTC)
		return t, true, err == nil
	}
	if s[10] != 'T' && s[10] != ' ' {
		return time.Time{}, false, false
	}
	body := s[:10] + "T" + s[11:]
	loc := time.UTC
	zone := ""
	rest := body[11:]
	if strings.HasSuffix(rest, "Z") {
		zone = "Z"
		body = body[:len(body)-1]
	} else if n := len(rest); n >= 6 && (rest[n-6] == '+' || rest[n-6] == '-') && rest[n-3] == ':' {
		zone = "off"
	} else if n := len(rest); n >= 5 && (rest[n-5] == '+' || rest[n-5] == '-') && strings.IndexByte(rest[n-4:], ':') < 0 && isDigits(rest[n-4:]) {
		zone = "off4"
		body = body[:len(body)-2] + ":" + body[len(body)-2:]
	}
	layouts := []string{"2006-01-02T15:04:05", "2006-01-02T15:04", "2006-01-02T15:04:05.999999999"}
	for _, l := range layouts {
		layout := l
		if zone == "off" || zone == "off4" {
			layout += "-07:00"
		}
		if t, err := time.ParseInLocation(layout, body, loc); err == nil {
			return t, zone == "", true
		}
	}
	return time.Time{}, false, false
}

func isDigits(s string) bool {
	if s == "" {
		return false
	}
	for _, c := range s {
		if c < '0' || c > '9' {
			return false
		}
	}
	return true
}

func ParseStamp(s string) (time.Time, bool) {
	t, _, ok := ParseISO(strings.Replace(s, "Z", "+00:00", 1))
	return t, ok
}

func ParseDate(s string) (time.Time, bool) {
	if len(s) < 10 {
		return time.Time{}, false
	}
	t, err := time.ParseInLocation("2006-01-02", s[:10], time.UTC)
	return t, err == nil
}

func DateStr(t time.Time) string {
	return t.Format("2006-01-02")
}

func Hex(v uint64) string { return strconv.FormatUint(v, 16) }

func Itoa(n int) string { return strconv.Itoa(n) }

const SpaceChars = " \t\n\r\x0b\x0c\x1c\x1d\x1e\x1f\u0085                 　"

func PtrInt64(v int64) *int64 { return &v }

func CommasFixed(n float64, decimals int) string {
	s := strconv.FormatFloat(math.Abs(n), 'f', decimals, 64)
	whole, frac, has := strings.Cut(s, ".")
	var b strings.Builder
	for i, c := range whole {
		if i > 0 && (len(whole)-i)%3 == 0 {
			b.WriteByte(',')
		}
		b.WriteRune(c)
	}
	out := b.String()
	if has {
		out += "." + frac
	}
	if n < 0 {
		out = "-" + out
	}
	return out
}

func Truthy(v any) bool {
	switch x := v.(type) {
	case nil:
		return false
	case string:
		return x != ""
	case bool:
		return x
	case float64:
		return x != 0
	case int:
		return x != 0
	case []any:
		return len(x) > 0
	case map[string]any:
		return len(x) > 0
	}
	return true
}

func OrStr(v any) string {
	if !Truthy(v) {
		return ""
	}
	return JSONStr(v)
}
