package app

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func newTestApp(t *testing.T) *App {
	t.Helper()
	a, err := New(Config{Home: t.TempDir(), OrdersLive: false, BindHost: "127.0.0.1"})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		a.setStop()
		a.st.Close()
	})
	return a
}

const testPort = 8765

func request(t *testing.T, a *App, method, path, body string, remote, host string, headers map[string]string) *httptest.ResponseRecorder {
	t.Helper()
	var r *http.Request
	if body == "" {
		r = httptest.NewRequest(method, path, nil)
	} else {
		r = httptest.NewRequest(method, path, strings.NewReader(body))
	}
	r.RemoteAddr = remote
	r.Host = host
	for k, v := range headers {
		r.Header.Set(k, v)
	}
	w := httptest.NewRecorder()
	a.handle(testPort).ServeHTTP(w, r)
	return w
}

func TestOnlyLoopbackIsAnswered(t *testing.T) {
	a := newTestApp(t)
	for _, remote := range []string{"127.0.0.1:51000", "[::1]:51000"} {
		w := request(t, a, http.MethodGet, "/api/nothing", "", remote, "127.0.0.1:8765", nil)
		if w.Code == 403 {
			t.Errorf("%s: loopback was refused", remote)
		}
	}
	for _, remote := range []string{"192.168.1.20:51000", "10.0.0.7:51000"} {
		w := request(t, a, http.MethodGet, "/api/nothing", "", remote, "127.0.0.1:8765", nil)
		if w.Code != 403 {
			t.Errorf("%s: code = %d, want 403", remote, w.Code)
		}
	}
}

func TestTheHostMustBeTheLoopbackAddressAndPort(t *testing.T) {
	a := newTestApp(t)
	if w := request(t, a, http.MethodGet, "/api/nothing", "", "127.0.0.1:51000", "127.0.0.1:8765", nil); w.Code == 403 {
		t.Error("the exact host was refused")
	}
	for _, host := range []string{"", "localhost:8765", "127.0.0.1:8766", "127.0.0.1", "evil.test", "127.0.0.1:8765,evil.test"} {
		w := request(t, a, http.MethodGet, "/api/nothing", "", "127.0.0.1:51000", host, nil)
		if w.Code != 403 {
			t.Errorf("host %q: code = %d, want 403", host, w.Code)
		}
	}
}

func TestAWriteNeedsSameOriginOrTheAppsOwnHeader(t *testing.T) {
	a := newTestApp(t)
	if w := request(t, a, http.MethodPost, "/api/nothing", "{}", "127.0.0.1:51000", "127.0.0.1:8765", nil); w.Code != 403 {
		t.Errorf("an unmarked write: code = %d, want 403", w.Code)
	}
	for _, h := range []map[string]string{{"Sec-Fetch-Site": "same-origin"}, {"Sec-Fetch-Site": "Same-Origin"}, {"X-Bagholder": "1"}} {
		if w := request(t, a, http.MethodPost, "/api/nothing", "{}", "127.0.0.1:51000", "127.0.0.1:8765", h); w.Code == 403 {
			t.Errorf("%v: the write was refused", h)
		}
	}
	for _, h := range []map[string]string{{"Sec-Fetch-Site": "cross-site"}, {"Sec-Fetch-Site": "none"}, {"X-Bagholder": "  "}} {
		if w := request(t, a, http.MethodPost, "/api/nothing", "{}", "127.0.0.1:51000", "127.0.0.1:8765", h); w.Code != 403 {
			t.Errorf("%v: code = %d, want 403", h, w.Code)
		}
	}
}

func TestAReadNeedsNoWriteMarking(t *testing.T) {
	a := newTestApp(t)
	if w := request(t, a, http.MethodGet, "/api/nothing", "", "127.0.0.1:51000", "127.0.0.1:8765", nil); w.Code == 403 {
		t.Errorf("a plain read was refused: %d", w.Code)
	}
}

func TestOptionsIsAnsweredWithForbidden(t *testing.T) {
	a := newTestApp(t)
	w := request(t, a, http.MethodOptions, "/api/status", "", "127.0.0.1:51000", "127.0.0.1:8765", map[string]string{"Sec-Fetch-Site": "same-origin"})
	if w.Code != 403 {
		t.Errorf("OPTIONS: code = %d, want 403", w.Code)
	}
}

func TestAnUnknownMethodIsNotImplemented(t *testing.T) {
	a := newTestApp(t)
	w := request(t, a, http.MethodPut, "/api/status", "", "127.0.0.1:51000", "127.0.0.1:8765", nil)
	if w.Code != 501 {
		t.Errorf("PUT: code = %d, want 501", w.Code)
	}
}

func TestABodyOverAMebibyteReadsAsEmpty(t *testing.T) {
	big := "{\"pad\":\"" + strings.Repeat("x", 1_048_576) + "\"}"
	r := httptest.NewRequest(http.MethodPost, "/api/nothing", strings.NewReader(big))
	if got := len(readJSON(r)); got != 0 {
		t.Errorf("a body of %d bytes read %d keys, want none", len(big), got)
	}
}

func TestABodyAtTheLimitIsStillRead(t *testing.T) {
	pad := strings.Repeat("x", 1_048_576-len(`{"pad":""}`))
	r := httptest.NewRequest(http.MethodPost, "/api/nothing", strings.NewReader(`{"pad":"`+pad+`"}`))
	if got := len(readJSON(r)); got != 1 {
		t.Errorf("a body of exactly 1 MiB read %d keys, want 1", got)
	}
}

func TestAnInvalidOrNonObjectBodyReadsAsEmpty(t *testing.T) {
	for _, body := range []string{"", "not json", "[1,2,3]", "\"text\"", "17", "null"} {
		r := httptest.NewRequest(http.MethodPost, "/api/nothing", strings.NewReader(body))
		if got := len(readJSON(r)); got != 0 {
			t.Errorf("body %q read %d keys, want none", body, got)
		}
	}
}

func TestAnObjectBodyIsRead(t *testing.T) {
	r := httptest.NewRequest(http.MethodPost, "/api/nothing", strings.NewReader(`{"symbol":"QNC","qty":3}`))
	body := readJSON(r)
	if body["symbol"] != "QNC" {
		t.Errorf("symbol = %v", body["symbol"])
	}
	if body["qty"] != 3.0 {
		t.Errorf("qty = %v", body["qty"])
	}
}
