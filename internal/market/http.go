package market

import (
	"bytes"
	"compress/gzip"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"errors"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/ProfessorBagholder/Bagholder/internal/store"
)

const (
	TimeoutSec      = 30
	HTTPPoolPerHost = 2
	HTTPRedirectMax = 5
	UA              = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
)

type HTTPError struct {
	URL  string
	Code int
	Msg  string
}

func (e *HTTPError) Error() string { return "HTTP Error " + strconv.Itoa(e.Code) + ": " + e.Msg }

func StatusOf(err error) int {
	var he *HTTPError
	if errors.As(err, &he) {
		return he.Code
	}
	return 0
}

var ErrBackingOff = errors.New("yahoo: backing off after 429")

var SourceLabels = []struct{ Key, Name string }{
	{"tmx", "TMX Money"}, {"yahoo", "Yahoo Finance"}, {"coinbase", "Coinbase"}, {"cboe", "Cboe"}, {"boc", "Bank of Canada"}, {"fred", "FRED"}, {"stooq", "Stooq"},
	{"finra", "FINRA"}, {"ciro", "CIRO"},
}

func SourceLabel(key string) string {
	for _, s := range SourceLabels {
		if s.Key == key {
			return s.Name
		}
	}
	return key
}

var sourceHosts = []struct{ Key, Needle string }{
	{"tmx", "tmx.com"}, {"yahoo", "yahoo.com"}, {"coinbase", "coinbase.com"}, {"cboe", "cboe.com"}, {"boc", "bankofcanada.ca"}, {"fred", "stlouisfed.org"},
	{"stooq", "stooq.com"}, {"finra", "finra.org"}, {"ciro", "ciro.ca"},
}

func SourceOfURL(raw string) string {
	u, err := url.Parse(raw)
	host := ""
	if err == nil {
		host = strings.ToLower(u.Host)
	}
	for _, s := range sourceHosts {
		if strings.Contains(host, s.Needle) {
			return s.Key
		}
	}
	if host == "" {
		return "other"
	}
	return host
}

func DescribeFailure(err error) string {
	code := StatusOf(err)
	if code == 429 {
		return "refused the request (too many)"
	}
	if code != 0 {
		return "answered with an error (" + strconv.Itoa(code) + ")"
	}
	if errors.Is(err, ErrBackingOff) {
		return "refused the request; asked again in ten minutes"
	}
	return "could not be reached"
}

type Health struct {
	Key   string `json:"key"`
	Name  string `json:"name"`
	OK    bool   `json:"ok"`
	At    string `json:"at"`
	Error string `json:"error"`
}

type Client struct {
	Store  *store.Store
	HTTP   *http.Client
	Now    func() time.Time
	health map[string]Health
	hmu    sync.Mutex

	yahooMu           sync.Mutex
	yahooNextAt       time.Time
	yahooBackoffUntil time.Time

	peekMu sync.Mutex
	peek   map[string]peekHit

	notes map[noteKey][]chainNote

	pendingMu sync.Mutex
	pending   map[string]bool

	refreshMu  sync.Mutex
	refreshing bool
}

type peekHit struct {
	at    time.Time
	quote PeekQuote
}

func NewClient(st *store.Store) *Client {
	return &Client{Store: st, HTTP: NewHTTPClient(), Now: func() time.Time { return time.Now().UTC() }, health: map[string]Health{}, peek: map[string]peekHit{}, notes: map[noteKey][]chainNote{}, pending: map[string]bool{}}
}

func caBundle() *x509.CertPool {
	pool, err := x509.SystemCertPool()
	if err != nil || pool == nil {
		pool = x509.NewCertPool()
	}
	for _, path := range []string{os.Getenv("SSL_CERT_FILE"), "/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/opt/homebrew/etc/openssl@3/cert.pem", "/usr/local/etc/openssl@3/cert.pem", "/opt/homebrew/etc/openssl@1.1/cert.pem"} {
		if path == "" {
			continue
		}
		if pem, err := os.ReadFile(path); err == nil {
			pool.AppendCertsFromPEM(pem)
		}
	}
	return pool
}

func NewHTTPClient() *http.Client {
	transport := &http.Transport{
		Proxy:               http.ProxyFromEnvironment,
		DialContext:         (&net.Dialer{Timeout: TimeoutSec * time.Second, KeepAlive: 30 * time.Second}).DialContext,
		TLSClientConfig:     &tls.Config{RootCAs: caBundle()},
		MaxIdleConnsPerHost: HTTPPoolPerHost,
		IdleConnTimeout:     90 * time.Second,
		TLSHandshakeTimeout: TimeoutSec * time.Second,
		ForceAttemptHTTP2:   true,
	}
	return &http.Client{
		Transport: transport,
		Timeout:   TimeoutSec * time.Second,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if req.Method != http.MethodGet || via[0].Method != http.MethodGet {
				return http.ErrUseLastResponse
			}
			if len(via) >= HTTPRedirectMax {
				return &HTTPError{URL: req.URL.String(), Code: 310, Msg: "too many redirects"}
			}
			return nil
		},
	}
}

func (c *Client) Clock() time.Time { return c.now() }

func (c *Client) now() time.Time {
	if c.Now != nil {
		return c.Now()
	}
	return time.Now().UTC()
}

func (c *Client) NoteSource(name string, ok bool, err error) {
	c.NoteSourceAt(name, ok, err, c.now())
}

func (c *Client) NoteSourceAt(name string, ok bool, err error, now time.Time) {
	c.hmu.Lock()
	defer c.hmu.Unlock()
	h := Health{Key: name, Name: SourceLabel(name), OK: ok, At: now.UTC().Format("2006-01-02T15:04:05Z")}
	if !ok {
		h.Error = DescribeFailure(err)
	}
	c.health[name] = h
}

func (c *Client) SourceHealth() []Health {
	c.hmu.Lock()
	defer c.hmu.Unlock()
	out := []Health{}
	for _, s := range SourceLabels {
		if h, ok := c.health[s.Key]; ok {
			out = append(out, h)
		}
	}
	return out
}

func (c *Client) ResetHealth() {
	c.hmu.Lock()
	c.health = map[string]Health{}
	c.hmu.Unlock()
}

func (c *Client) fetch(rawURL string, headers map[string]string, method string, body []byte) ([]byte, error) {
	var reader io.Reader
	if body != nil {
		reader = bytes.NewReader(body)
	}
	req, err := http.NewRequest(method, rawURL, reader)
	if err != nil {
		return nil, err
	}
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	if body != nil {
		req.ContentLength = int64(len(body))
	}
	resp, err := c.HTTP.Do(req)
	if err != nil {
		var he *HTTPError
		if errors.As(err, &he) {
			return nil, he
		}
		return nil, err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 64<<20))
	if err != nil {
		return nil, err
	}
	if resp.StatusCode >= 400 {
		return nil, &HTTPError{URL: rawURL, Code: resp.StatusCode, Msg: "HTTP " + strconv.Itoa(resp.StatusCode)}
	}
	if resp.StatusCode >= 300 {
		return nil, &HTTPError{URL: rawURL, Code: resp.StatusCode, Msg: "HTTP " + strconv.Itoa(resp.StatusCode)}
	}
	return gunzipIfNeeded(raw), nil
}

func GunzipIfNeeded(raw []byte) []byte { return gunzipIfNeeded(raw) }

func gunzipIfNeeded(raw []byte) []byte {
	if len(raw) >= 2 && raw[0] == 0x1f && raw[1] == 0x8b {
		zr, err := gzip.NewReader(bytes.NewReader(raw))
		if err == nil {
			if out, err := io.ReadAll(zr); err == nil {
				return out
			}
		}
	}
	return raw
}

var defaultGetHeaders = map[string]string{"User-Agent": UA, "Accept": "text/csv,application/json,*/*;q=0.8"}

func (c *Client) GetText(rawURL string, headers map[string]string) (string, error) {
	if headers == nil {
		headers = defaultGetHeaders
	}
	raw, err := c.fetch(rawURL, headers, http.MethodGet, nil)
	if err != nil {
		if StatusOf(err) != 404 {
			c.NoteSource(SourceOfURL(rawURL), false, err)
		}
		return "", err
	}
	c.NoteSource(SourceOfURL(rawURL), true, nil)
	return string(raw), nil
}

func (c *Client) PostJSON(rawURL string, payload any, headers map[string]string) (map[string]any, error) {
	body, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	hdrs := map[string]string{"User-Agent": UA, "Content-Type": "application/json", "Accept": "*/*"}
	for k, v := range headers {
		hdrs[k] = v
	}
	hdrs["Content-Length"] = strconv.Itoa(len(body))
	raw, err := c.fetch(rawURL, hdrs, http.MethodPost, body)
	if err != nil {
		c.NoteSource(SourceOfURL(rawURL), false, err)
		return nil, err
	}
	c.NoteSource(SourceOfURL(rawURL), true, nil)
	var out map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		var anyv any
		if err2 := json.Unmarshal(raw, &anyv); err2 != nil {
			return nil, err2
		}
		return nil, nil
	}
	return out, nil
}

func sortedKeys(m map[string]float64) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func stamp(t time.Time) string { return t.UTC().Format("2006-01-02T15:04:05Z") }

func parseStamp(s string) (time.Time, bool) {
	s = strings.Replace(s, "Z", "+00:00", 1)
	t, err := time.Parse("2006-01-02T15:04:05-07:00", s)
	if err != nil {
		t, err = time.Parse("2006-01-02T15:04:05.999999-07:00", s)
	}
	if err != nil {
		return time.Time{}, false
	}
	return t, true
}

func AgeOf(stampText string, now time.Time) (time.Duration, bool) { return ageOf(stampText, now) }

func ageOf(stampText string, now time.Time) (time.Duration, bool) {
	if stampText == "" {
		return 0, false
	}
	t, ok := parseStamp(stampText)
	if !ok {
		return 0, false
	}
	return now.Sub(t), true
}

func (c *Client) FetchRaw(rawURL string, headers map[string]string) ([]byte, error) {
	if headers == nil {
		headers = defaultGetHeaders
	}
	return c.fetch(rawURL, headers, http.MethodGet, nil)
}

func (c *Client) PostJSONList(rawURL string, payload any, headers map[string]string) ([]map[string]any, error) {
	body, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}
	hdrs := map[string]string{"User-Agent": UA, "Content-Type": "application/json", "Accept": "*/*"}
	for k, v := range headers {
		hdrs[k] = v
	}
	hdrs["Content-Length"] = strconv.Itoa(len(body))
	raw, err := c.fetch(rawURL, hdrs, http.MethodPost, body)
	if err != nil {
		c.NoteSource(SourceOfURL(rawURL), false, err)
		return nil, err
	}
	c.NoteSource(SourceOfURL(rawURL), true, nil)
	var out []map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		return nil, err
	}
	return out, nil
}

func (c *Client) FetchRawWithType(rawURL string, headers map[string]string) ([]byte, string, error) {
	req, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, "", err
	}
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	resp, err := c.HTTP.Do(req)
	if err != nil {
		return nil, "", err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 256<<20))
	if err != nil {
		return nil, "", err
	}
	if resp.StatusCode >= 400 {
		return nil, "", &HTTPError{URL: rawURL, Code: resp.StatusCode, Msg: "HTTP " + strconv.Itoa(resp.StatusCode)}
	}
	if strings.EqualFold(resp.Header.Get("Content-Encoding"), "gzip") {
		raw = gunzipIfNeeded(raw)
	}
	ct := resp.Header.Get("Content-Type")
	if i := strings.Index(ct, ";"); i >= 0 {
		ct = strings.TrimSpace(ct[:i])
	}
	return raw, strings.ToLower(ct), nil
}
