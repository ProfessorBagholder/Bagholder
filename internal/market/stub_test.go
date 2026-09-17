package market

import (
	"encoding/json"
	"errors"
	"io"
	"math"
	"net/http"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

var errNoRoute = errors.New("no route")

type stubCall struct {
	Method string
	URL    string
	Body   string
	Header http.Header
}

type stub struct {
	c      *Client
	mu     sync.Mutex
	calls  []stubCall
	answer func(call stubCall) (int, string, error)
}

func (s *stub) RoundTrip(req *http.Request) (*http.Response, error) {
	body := ""
	if req.Body != nil {
		raw, _ := io.ReadAll(req.Body)
		body = string(raw)
	}
	call := stubCall{Method: req.Method, URL: req.URL.String(), Body: body, Header: req.Header.Clone()}
	s.mu.Lock()
	s.calls = append(s.calls, call)
	answer := s.answer
	s.mu.Unlock()
	if strings.Contains(req.URL.Host, "yahoo.com") {
		s.c.yahooNextAt = time.Time{}
	}
	if answer == nil {
		return nil, errNoRoute
	}
	status, text, err := answer(call)
	if err != nil {
		return nil, err
	}
	return &http.Response{StatusCode: status, Status: strconv.Itoa(status) + " " + http.StatusText(status), Header: http.Header{}, Body: io.NopCloser(strings.NewReader(text)), Request: req}, nil
}

func (s *stub) set(answer func(call stubCall) (int, string, error)) {
	s.mu.Lock()
	s.answer = answer
	s.mu.Unlock()
}

func (s *stub) reset() {
	s.mu.Lock()
	s.calls = nil
	s.mu.Unlock()
}

func (s *stub) all() []stubCall {
	s.mu.Lock()
	defer s.mu.Unlock()
	return append([]stubCall{}, s.calls...)
}

func (s *stub) count() int { return len(s.all()) }

func (s *stub) urls(needle string) []string {
	out := []string{}
	for _, c := range s.all() {
		if strings.Contains(c.URL, needle) {
			out = append(out, c.URL)
		}
	}
	return out
}

func (s *stub) gets() int {
	n := 0
	for _, c := range s.all() {
		if c.Method == http.MethodGet {
			n++
		}
	}
	return n
}

type tmxPost struct {
	Op   string
	Vars map[string]any
}

func (p tmxPost) Sym() string {
	s, _ := p.Vars["symbol"].(string)
	return s
}

func (p tmxPost) Var(name string) string {
	s, _ := p.Vars[name].(string)
	return s
}

func parseTMX(body string) (tmxPost, bool) {
	var p struct {
		Op   string         `json:"operationName"`
		Vars map[string]any `json:"variables"`
	}
	if json.Unmarshal([]byte(body), &p) != nil || p.Op == "" {
		return tmxPost{}, false
	}
	return tmxPost{Op: p.Op, Vars: p.Vars}, true
}

func (s *stub) tmxPosts() []tmxPost {
	out := []tmxPost{}
	for _, c := range s.all() {
		if c.URL != TMXURL {
			continue
		}
		if p, ok := parseTMX(c.Body); ok {
			out = append(out, p)
		}
	}
	return out
}

func (s *stub) tmxOps() [][2]string {
	out := [][2]string{}
	for _, p := range s.tmxPosts() {
		out = append(out, [2]string{p.Op, p.Sym()})
	}
	return out
}

func (s *stub) tmxSymbols(op string) []string {
	out := []string{}
	for _, p := range s.tmxPosts() {
		if p.Op == op {
			out = append(out, p.Sym())
		}
	}
	return out
}

func newTestClient(t *testing.T, now time.Time) (*Client, *stub) {
	t.Helper()
	st := store.MustOpen(t.TempDir())
	if err := st.Ensure(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { st.Close() })
	c := NewClient(st)
	s := &stub{c: c}
	c.HTTP = &http.Client{Transport: s}
	if !now.IsZero() {
		c.Now = func() time.Time { return now }
	}
	return c, s
}

func jsonText(v any) string {
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	return string(b)
}

func obj(pairs ...any) map[string]any {
	out := map[string]any{}
	for i := 0; i+1 < len(pairs); i += 2 {
		out[pairs[i].(string)] = pairs[i+1]
	}
	return out
}

func utc(y int, m time.Month, d, h, mi, s int) time.Time {
	return time.Date(y, m, d, h, mi, s, 0, time.UTC)
}

func rec(symbol, exchange, currency, kind string) Rec {
	return Rec{Symbol: symbol, Exchange: exchange, Currency: currency, Kind: kind}
}

func ptr(f float64) *float64 { return &f }

func fv(p *float64) any {
	if p == nil {
		return nil
	}
	return *p
}

func price(q *store.Quote) any {
	if q == nil {
		return nil
	}
	return fv(q.Price)
}

func eq(t *testing.T, got, want any, msg string) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		if msg != "" {
			t.Fatalf("%s: got %#v, want %#v", msg, got, want)
		}
		t.Fatalf("got %#v, want %#v", got, want)
	}
}

func near(t *testing.T, got any, want float64, msg string) {
	t.Helper()
	f, ok := got.(float64)
	if !ok || math.Abs(f-want) > 1e-7 {
		if msg != "" {
			t.Fatalf("%s: got %#v, want %v", msg, got, want)
		}
		t.Fatalf("got %#v, want %v", got, want)
	}
}

func keysOf[V any](m map[string]V) []string {
	out := []string{}
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func uniqSorted(list []string) []string {
	seen := map[string]bool{}
	for _, s := range list {
		seen[s] = true
	}
	return keysOf(seen)
}

func dailyRows(bars []Daily) [][]any {
	out := [][]any{}
	for _, b := range bars {
		out = append(out, []any{b.Date, fv(b.Open), fv(b.High), fv(b.Low), b.Close, fv(b.Volume)})
	}
	return out
}

func barRows(bars []Bar) [][]any {
	out := [][]any{}
	for _, b := range bars {
		out = append(out, []any{b.Time, fv(b.Open), fv(b.High), fv(b.Low), b.Close, fv(b.Volume)})
	}
	return out
}

func dates(bars []Daily) []string {
	out := []string{}
	for _, b := range bars {
		out = append(out, b.Date)
	}
	return out
}

func closes(bars []Bar) []float64 {
	out := []float64{}
	for _, b := range bars {
		out = append(out, b.Close)
	}
	return out
}

func tmxRow(day string, open, high, low, close, volume float64) map[string]any {
	return obj("dateTime", day+"T16:00:00-04:00", "open", open, "high", high, "low", low, "close", close, "volume", volume)
}

func tmxSeries(rows ...map[string]any) string {
	list := []any{}
	for _, r := range rows {
		list = append(list, r)
	}
	return jsonText(obj("data", obj("getTimeSeriesData", list)))
}

func tmxIntraday(rows ...map[string]any) string {
	list := []any{}
	for _, r := range rows {
		list = append(list, r)
	}
	return jsonText(obj("data", obj("intraday", list)))
}

func minuteRow(day, hhmm, off string, open, high, low, close, volume float64) map[string]any {
	return obj("dateTime", day+"T"+hhmm+":00"+off, "open", open, "high", high, "low", low, "close", close, "volume", volume)
}
