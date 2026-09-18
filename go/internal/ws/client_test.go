package ws

import (
	"bytes"
	"compress/gzip"
	"io"
	"net/http"
	"os"
	"strings"
	"testing"
)

var fakeClientID = strings.Repeat("ab", 32)

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(req *http.Request) (*http.Response, error) { return f(req) }

func httpResponse(req *http.Request, status int, body []byte, headers map[string]string) *http.Response {
	resp := &http.Response{
		StatusCode:    status,
		Proto:         "HTTP/1.1",
		ProtoMajor:    1,
		ProtoMinor:    1,
		Header:        http.Header{},
		Body:          io.NopCloser(bytes.NewReader(body)),
		ContentLength: int64(len(body)),
		Request:       req,
	}
	for k, v := range headers {
		resp.Header.Set(k, v)
	}
	return resp
}

func newTestClient(home string, fn func(*http.Request) *http.Response) (*Client, *string) {
	c := NewClient(home)
	c.HTTP.Transport = roundTripFunc(func(req *http.Request) (*http.Response, error) { return fn(req), nil })
	last := new(string)
	c.OnError = func(msg string) { *last = msg }
	return c, last
}

type requestCounter struct {
	scrapes int
	posts   int
}

func countingTransport(t *testing.T, counter *requestCounter, post func(*http.Request) *http.Response) func(*http.Request) *http.Response {
	return func(req *http.Request) *http.Response {
		url := req.URL.String()
		if url == LoginURL || (strings.Contains(url, "app-") && strings.Contains(url, ".js")) {
			counter.scrapes++
			return httpResponse(req, 200, []byte(loginHTML("https://assets.wealthsimple.com/app-abc123.js")), nil)
		}
		counter.posts++
		if post != nil {
			return post(req)
		}
		t.Errorf("unexpected request %s %s", req.Method, url)
		return httpResponse(req, 500, nil, nil)
	}
}

func loginHTML(jsURL string) string {
	return `<html><script src="` + jsURL + `"></script></html>`
}

func appJS(clientID string) string {
	return `var cfg={production:{env:"prod",clientId:"` + clientID + `"}};`
}

func gzipBytes(t *testing.T, data []byte) []byte {
	t.Helper()
	var buf bytes.Buffer
	zw := gzip.NewWriter(&buf)
	if _, err := zw.Write(data); err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}
	return buf.Bytes()
}

func urlopenForJS(jsBody, htmlBody []byte, jsHeaders, htmlHeaders map[string]string) func(*http.Request) *http.Response {
	if htmlBody == nil {
		htmlBody = []byte(loginHTML("https://assets.wealthsimple.com/app-abc123.js"))
	}
	return func(req *http.Request) *http.Response {
		url := req.URL.String()
		if strings.Contains(url, "app-") && strings.Contains(url, ".js") {
			return httpResponse(req, 200, jsBody, jsHeaders)
		}
		return httpResponse(req, 200, htmlBody, htmlHeaders)
	}
}

func fileText(t *testing.T, path string) string {
	t.Helper()
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("%s: %v", path, err)
	}
	return strings.TrimSpace(string(raw))
}

func fileExists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func TestScrapeClientIDFromGzipJS(t *testing.T) {
	js := appJS(fakeClientID)
	gz := gzipBytes(t, []byte(js))
	if gz[0] != 0x1f || gz[1] != 0x8b {
		t.Fatalf("gzip magic: got %x", gz[:2])
	}
	c, _ := newTestClient(t.TempDir(), urlopenForJS(gz, nil, map[string]string{"Content-Encoding": "gzip"}, nil))
	cid := c.ScrapeClientID()
	if cid != fakeClientID {
		t.Errorf("scrape_client_id: got %q, want %q", cid, fakeClientID)
	}
	if got := fileText(t, c.Files.ClientIDPath()); got != fakeClientID {
		t.Errorf("CLIENT_ID_PATH: got %q, want %q", got, fakeClientID)
	}
}

func TestScrapeClientIDFromUncompressedJS(t *testing.T) {
	js := appJS(fakeClientID)
	c, _ := newTestClient(t.TempDir(), urlopenForJS([]byte(js), nil, nil, nil))
	cid := c.ScrapeClientID()
	if cid != fakeClientID {
		t.Errorf("scrape_client_id: got %q, want %q", cid, fakeClientID)
	}
}

func TestScrapeClientIDFromGzipLoginHTML(t *testing.T) {
	htmlGz := gzipBytes(t, []byte(loginHTML("https://assets.wealthsimple.com/app-abc123.js")))
	js := appJS(fakeClientID)
	c, _ := newTestClient(t.TempDir(), urlopenForJS([]byte(js), htmlGz, nil, map[string]string{"Content-Encoding": "gzip"}))
	cid := c.ScrapeClientID()
	if cid != fakeClientID {
		t.Errorf("scrape_client_id: got %q, want %q", cid, fakeClientID)
	}
}

func TestSessionClientIDIsWrittenToDisk(t *testing.T) {
	counter := &requestCounter{}
	c, _ := newTestClient(t.TempDir(), countingTransport(t, counter, nil))
	found := c.ClientIDFor(Session{"client_id": fakeClientID})
	if found != fakeClientID {
		t.Errorf("client_id_for: got %q, want %q", found, fakeClientID)
	}
	if !fileExists(c.Files.ClientIDPath()) {
		t.Fatal("CLIENT_ID_PATH does not exist")
	}
	if got := fileText(t, c.Files.ClientIDPath()); got != fakeClientID {
		t.Errorf("CLIENT_ID_PATH: got %q, want %q", got, fakeClientID)
	}
}

func TestTokenInfoUIDIsStored(t *testing.T) {
	counter := &requestCounter{}
	c, _ := newTestClient(t.TempDir(), countingTransport(t, counter, nil))
	sess := Session{"access_token": "tok", "refresh_token": "r"}
	info := map[string]any{"application_uid": fakeClientID}
	found := c.ApplyTokenInfoClientID(sess, info)
	if found != fakeClientID {
		t.Errorf("apply_token_info_client_id: got %q, want %q", found, fakeClientID)
	}
	if sess.Str("client_id") != fakeClientID {
		t.Errorf("sess client_id: got %q, want %q", sess.Str("client_id"), fakeClientID)
	}
	if got := fileText(t, c.Files.ClientIDPath()); got != fakeClientID {
		t.Errorf("CLIENT_ID_PATH: got %q, want %q", got, fakeClientID)
	}
	sess2 := Session{"access_token": "tok"}
	nested := ClientIDFromTokenInfo(map[string]any{"application": map[string]any{"uid": fakeClientID}})
	if nested != fakeClientID {
		t.Errorf("client_id_from_token_info: got %q, want %q", nested, fakeClientID)
	}
	c.ApplyTokenInfoClientID(sess2, map[string]any{"application": map[string]any{"uid": fakeClientID}})
	if sess2.Str("client_id") != fakeClientID {
		t.Errorf("sess2 client_id: got %q, want %q", sess2.Str("client_id"), fakeClientID)
	}
	if counter.scrapes != 0 || counter.posts != 0 {
		t.Errorf("requests made: %d scrapes, %d posts, want none", counter.scrapes, counter.posts)
	}
}

func TestRefreshSessionWithoutClientIDDoesNotScrapeOrPost(t *testing.T) {
	counter := &requestCounter{}
	c, lastErr := newTestClient(t.TempDir(), countingTransport(t, counter, nil))
	c.Files.SaveSession(Session{"refresh_token": "r"})
	if fileExists(c.Files.ClientIDPath()) {
		t.Fatal("CLIENT_ID_PATH exists before the refresh")
	}
	ok := c.RefreshSession(Session{"refresh_token": "r"}, true)
	if ok {
		t.Error("refresh_session: got True, want False")
	}
	if counter.scrapes != 0 {
		t.Errorf("scrape_client_id was called %d times", counter.scrapes)
	}
	if counter.posts != 0 {
		t.Errorf("_http_json was called %d times", counter.posts)
	}
	if *lastErr != "session has no client id" {
		t.Errorf("error: got %q, want %q", *lastErr, "session has no client id")
	}
	if !fileExists(c.Files.SessionPath()) {
		t.Error("SESSION_PATH does not exist")
	}
	if got := c.Files.LoadSession().Str("refresh_token"); got != "r" {
		t.Errorf("saved refresh_token: got %q, want r", got)
	}
}

func TestRefreshSessionUsesCachedClientIDFile(t *testing.T) {
	counter := &requestCounter{}
	c, _ := newTestClient(t.TempDir(), countingTransport(t, counter, func(req *http.Request) *http.Response {
		if req.Method != http.MethodPost || req.URL.String() != OAuth+"/token" {
			t.Errorf("unexpected request %s %s", req.Method, req.URL)
		}
		return httpResponse(req, 200, []byte(`{"access_token":"tok","expires_in":3600}`), nil)
	}))
	c.Files.SaveClientID(fakeClientID)
	sess := Session{"refresh_token": "r"}
	ok := c.RefreshSession(sess, true)
	if !ok {
		t.Fatal("refresh_session: got False, want True")
	}
	if counter.scrapes != 0 {
		t.Errorf("scrape_client_id was called %d times", counter.scrapes)
	}
	if counter.posts != 1 {
		t.Errorf("_http_json was called %d times, want once", counter.posts)
	}
	if sess.Str("client_id") != fakeClientID {
		t.Errorf("sess client_id: got %q, want %q", sess.Str("client_id"), fakeClientID)
	}
	expiresAt, isStr := sess["expires_at"].(string)
	if !isStr {
		t.Fatalf("expires_at: got %T %v, want str", sess["expires_at"], sess["expires_at"])
	}
	if !strings.Contains(expiresAt, "T") {
		t.Errorf("expires_at %q has no T", expiresAt)
	}
	if !strings.HasSuffix(expiresAt, "Z") {
		t.Errorf("expires_at %q does not end with Z", expiresAt)
	}
}

func TestRefreshSessionSetsHTTPAndOAuthError(t *testing.T) {
	c, lastErr := newTestClient(t.TempDir(), func(req *http.Request) *http.Response {
		return httpResponse(req, 401, []byte(`{"error":"invalid_client"}`), nil)
	})
	sess := Session{"refresh_token": "r", "client_id": fakeClientID}
	c.Files.SaveSession(sess)
	ok := c.RefreshSession(sess, true)
	if ok {
		t.Error("refresh_session: got True, want False")
	}
	err := *lastErr
	if !strings.Contains(err, "HTTP 401") {
		t.Errorf("error %q lacks HTTP 401", err)
	}
	if !strings.Contains(err, "invalid_client") {
		t.Errorf("error %q lacks invalid_client", err)
	}
	if !strings.HasPrefix(err, "Wealthsimple token refresh HTTP 401") {
		t.Errorf("error %q does not start with %q", err, "Wealthsimple token refresh HTTP 401")
	}
	if strings.Contains(err, fakeClientID) {
		t.Errorf("error %q carries the client id", err)
	}
	for _, word := range strings.Fields(err) {
		if word == "r" {
			t.Errorf("error %q carries the refresh token", err)
		}
	}
	if !fileExists(c.Files.SessionPath()) {
		t.Error("SESSION_PATH does not exist")
	}
	if got := c.Files.LoadSession().Str("refresh_token"); got != "r" {
		t.Errorf("saved refresh_token: got %q, want r", got)
	}
}

func TestRefreshSessionSetsHTTPError(t *testing.T) {
	c, lastErr := newTestClient(t.TempDir(), func(req *http.Request) *http.Response {
		return httpResponse(req, 400, []byte(`{"error":"invalid_grant"}`), nil)
	})
	sess := Session{"refresh_token": "r", "client_id": fakeClientID}
	c.Files.SaveSession(sess)
	ok := c.RefreshSession(sess, true)
	if ok {
		t.Error("refresh_session: got True, want False")
	}
	if *lastErr != RefusedLogin {
		t.Errorf("error: got %q, want %q", *lastErr, RefusedLogin)
	}
	if !fileExists(c.Files.SessionPath()) {
		t.Error("SESSION_PATH does not exist")
	}
	if got := c.Files.LoadSession().Str("refresh_token"); got != "r" {
		t.Errorf("saved refresh_token: got %q, want r", got)
	}
}

func TestRefreshSessionSetsOAuthErrorText(t *testing.T) {
	c, lastErr := newTestClient(t.TempDir(), func(req *http.Request) *http.Response {
		return httpResponse(req, 200, []byte(`{"error":"invalid_grant"}`), nil)
	})
	sess := Session{"refresh_token": "r", "client_id": fakeClientID}
	ok := c.RefreshSession(sess, true)
	if ok {
		t.Error("refresh_session: got True, want False")
	}
	if *lastErr != RefusedLogin {
		t.Errorf("error: got %q, want %q", *lastErr, RefusedLogin)
	}
}

func TestHTTPJSONInvalidBodyReturnsErrorDict(t *testing.T) {
	c, _ := newTestClient(t.TempDir(), func(req *http.Request) *http.Response {
		return httpResponse(req, 200, []byte("not-json{"), nil)
	})
	data := c.HTTPJSON(http.MethodGet, "https://example.test/token", nil, nil, 0)
	if data["error"] != "invalid_json" {
		t.Errorf("error: got %v, want invalid_json", data["error"])
	}
	if _, ok := data["_http_status"]; !ok {
		t.Error("_http_status missing")
	}
}

func TestHTTPJSONReadsGzipJSON(t *testing.T) {
	raw := gzipBytes(t, []byte(`{"access_token":"tok","expires_in":3600}`))
	c, _ := newTestClient(t.TempDir(), func(req *http.Request) *http.Response {
		return httpResponse(req, 200, raw, map[string]string{"Content-Encoding": "gzip"})
	})
	data := c.HTTPJSON(http.MethodPost, "https://example.test/token", map[string]any{"grant_type": "refresh_token"}, nil, 0)
	if data["access_token"] != "tok" {
		t.Errorf("access_token: got %v, want tok", data["access_token"])
	}
	if _, ok := data["_http_status"]; ok {
		t.Errorf("_http_status present: %v", data["_http_status"])
	}
}
