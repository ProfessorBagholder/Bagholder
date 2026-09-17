package app

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode"

	"github.com/ProfessorBagholder/Bagholder/internal/py"
	"github.com/ProfessorBagholder/Bagholder/internal/ws"
)

const (
	captureCallSec  = 2.0
	windowCheckSec  = 0.5
	captureEverySec = 1.5
)

var shotEverySec = 700 * time.Millisecond

func secs(f float64) time.Duration { return time.Duration(f * float64(time.Second)) }

var macBrowsers = []string{
	"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
	"/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
	"/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
}

func FindChrome() string {
	if explicit := strings.TrimSpace(os.Getenv("BAGHOLDER_CHROME")); explicit != "" && isFile(explicit) {
		return explicit
	}
	if isDarwin() {
		for _, p := range macBrowsers {
			if isFile(p) {
				return p
			}
		}
	}
	if p := which("google-chrome", "google-chrome-stable", "brave-browser", "brave-browser-stable", "brave", "chromium", "chromium-browser", "microsoft-edge", "msedge", "chrome"); p != "" {
		return p
	}
	extras := append([]string{}, macBrowsers...)
	pf := os.Getenv("PROGRAMFILES")
	if pf == "" {
		pf = `C:\Program Files`
	}
	pf86 := os.Getenv("PROGRAMFILES(X86)")
	if pf86 == "" {
		pf86 = `C:\Program Files (x86)`
	}
	local := os.Getenv("LOCALAPPDATA")
	extras = append(extras,
		filepath.Join(pf, "Google", "Chrome", "Application", "chrome.exe"),
		filepath.Join(pf86, "Google", "Chrome", "Application", "chrome.exe"),
		filepath.Join(local, "Google", "Chrome", "Application", "chrome.exe"),
		filepath.Join(pf, "BraveSoftware", "Brave-Browser", "Application", "brave.exe"),
		filepath.Join(pf86, "BraveSoftware", "Brave-Browser", "Application", "brave.exe"),
		filepath.Join(local, "BraveSoftware", "Brave-Browser", "Application", "brave.exe"),
		filepath.Join(pf, "Microsoft", "Edge", "Application", "msedge.exe"),
		filepath.Join(pf86, "Microsoft", "Edge", "Application", "msedge.exe"),
	)
	for _, p := range extras {
		if isFile(p) {
			return p
		}
	}
	return ""
}

var cdpHTTP = &http.Client{Transport: &http.Transport{Proxy: nil, DisableKeepAlives: true}}

func cdpGet(port int, path string, timeout time.Duration) ([]byte, error) {
	req, err := http.NewRequest(http.MethodGet, "http://127.0.0.1:"+strconv.Itoa(port)+path, nil)
	if err != nil {
		return nil, err
	}
	req.Host = "127.0.0.1:" + strconv.Itoa(port)
	client := *cdpHTTP
	client.Timeout = timeout
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	if resp.StatusCode >= 400 {
		return nil, fmt.Errorf("HTTP %d", resp.StatusCode)
	}
	return io.ReadAll(io.LimitReader(resp.Body, 8<<20))
}

func cdpList(port int, timeout time.Duration) []map[string]any {
	for _, path := range []string{"/json/list", "/json"} {
		raw, err := cdpGet(port, path, timeout)
		if err != nil || len(raw) == 0 {
			continue
		}
		var data []any
		if err := json.Unmarshal(raw, &data); err != nil {
			continue
		}
		out := make([]map[string]any, 0, len(data))
		for _, item := range data {
			if m, ok := item.(map[string]any); ok {
				out = append(out, m)
			} else {
				out = append(out, nil)
			}
		}
		return out
	}
	return nil
}

func maskWS(data, key []byte) []byte {
	out := make([]byte, len(data))
	for i, b := range data {
		out[i] = b ^ key[i%4]
	}
	return out
}

type miniWS struct {
	conn   net.Conn
	buf    []byte
	nextID int
	mu     sync.Mutex
}

var errWSTimeout = errors.New("ws read timeout")
var errWSClosed = errors.New("ws closed")

func (w *miniWS) close() {
	_ = w.sendFrame(0x8, nil)
	_ = w.conn.Close()
}

func (w *miniWS) sendFrame(opcode byte, payload []byte) error {
	key := make([]byte, 4)
	_, _ = rand.Read(key)
	masked := maskWS(payload, key)
	n := len(payload)
	header := []byte{0x80 | (opcode & 0x0F)}
	switch {
	case n < 126:
		header = append(header, 0x80|byte(n))
	case n < 65536:
		header = append(header, 0x80|126)
		header = binary.BigEndian.AppendUint16(header, uint16(n))
	default:
		header = append(header, 0x80|127)
		header = binary.BigEndian.AppendUint64(header, uint64(n))
	}
	header = append(header, key...)
	w.mu.Lock()
	defer w.mu.Unlock()
	_, err := w.conn.Write(append(header, masked...))
	return err
}

func (w *miniWS) sendText(text string) error { return w.sendFrame(0x1, []byte(text)) }

func (w *miniWS) recvExact(n int, deadline time.Time) ([]byte, error) {
	if n <= 0 {
		return nil, nil
	}
	for len(w.buf) < n {
		remain := time.Until(deadline)
		if remain <= 0 {
			return nil, errWSTimeout
		}
		if remain < 50*time.Millisecond {
			remain = 50 * time.Millisecond
		}
		_ = w.conn.SetReadDeadline(time.Now().Add(remain))
		chunk := make([]byte, 65536)
		k, err := w.conn.Read(chunk)
		if k > 0 {
			w.buf = append(w.buf, chunk[:k]...)
		}
		if err != nil {
			if k == 0 {
				var ne net.Error
				if errors.As(err, &ne) && ne.Timeout() {
					return nil, errWSTimeout
				}
				return nil, errWSClosed
			}
		}
	}
	data := w.buf[:n:n]
	w.buf = w.buf[n:]
	return data, nil
}

func (w *miniWS) readFrame(deadline time.Time) (bool, byte, []byte, error) {
	b1, err := w.recvExact(1, deadline)
	if err != nil {
		return false, 0, nil, err
	}
	b2, err := w.recvExact(1, deadline)
	if err != nil {
		return false, 0, nil, err
	}
	fin := b1[0]&0x80 != 0
	opcode := b1[0] & 0x0F
	masked := b2[0]&0x80 != 0
	length := uint64(b2[0] & 0x7F)
	if length == 126 {
		raw, err := w.recvExact(2, deadline)
		if err != nil {
			return false, 0, nil, err
		}
		length = uint64(binary.BigEndian.Uint16(raw))
	} else if length == 127 {
		raw, err := w.recvExact(8, deadline)
		if err != nil {
			return false, 0, nil, err
		}
		length = binary.BigEndian.Uint64(raw)
	}
	var key []byte
	if masked {
		key, err = w.recvExact(4, deadline)
		if err != nil {
			return false, 0, nil, err
		}
	}
	var payload []byte
	if length > 0 {
		payload, err = w.recvExact(int(length), deadline)
		if err != nil {
			return false, 0, nil, err
		}
	}
	if key != nil {
		payload = maskWS(payload, key)
	}
	return fin, opcode, payload, nil
}

func (w *miniWS) recvMessage(timeout time.Duration) (byte, []byte, error) {
	deadline := time.Now().Add(timeout)
	var fragments [][]byte
	var started byte
	for {
		if time.Until(deadline) <= 0 {
			return 0, nil, errWSTimeout
		}
		fin, opcode, payload, err := w.readFrame(deadline)
		if err != nil {
			return 0, nil, err
		}
		switch opcode {
		case 0x8:
			return 0, nil, errWSClosed
		case 0x9:
			_ = w.sendFrame(0xA, payload)
			continue
		case 0xA:
			continue
		case 0x1, 0x2:
			started = opcode
			fragments = [][]byte{payload}
			if fin {
				return started, joinFragments(fragments), nil
			}
		case 0x0:
			fragments = append(fragments, payload)
			if fin {
				if started == 0 {
					started = 0x1
				}
				return started, joinFragments(fragments), nil
			}
		}
	}
}

func joinFragments(parts [][]byte) []byte {
	var out []byte
	for _, p := range parts {
		out = append(out, p...)
	}
	return out
}

func wsConnect(wsURL string, timeout time.Duration) (*miniWS, error) {
	parsed, err := url.Parse(wsURL)
	if err != nil {
		return nil, err
	}
	host := "127.0.0.1"
	port := parsed.Port()
	if port == "" {
		if parsed.Scheme == "wss" {
			port = "443"
		} else {
			port = "80"
		}
	}
	path := parsed.Path
	if path == "" {
		path = "/"
	}
	if parsed.RawQuery != "" {
		path += "?" + parsed.RawQuery
	}
	conn, err := net.DialTimeout("tcp", net.JoinHostPort(host, port), timeout)
	if err != nil {
		return nil, err
	}
	keyRaw := make([]byte, 16)
	_, _ = rand.Read(keyRaw)
	key := base64.StdEncoding.EncodeToString(keyRaw)
	req := "GET " + path + " HTTP/1.1\r\nHost: " + host + ":" + port + "\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: " + key + "\r\nSec-WebSocket-Version: 13\r\nOrigin: http://127.0.0.1\r\n\r\n"
	if _, err := conn.Write([]byte(req)); err != nil {
		conn.Close()
		return nil, err
	}
	var buf []byte
	deadline := time.Now().Add(timeout)
	for !strings.Contains(string(buf), "\r\n\r\n") {
		remain := time.Until(deadline)
		if remain <= 0 {
			conn.Close()
			return nil, errors.New("ws handshake timeout")
		}
		if remain < 50*time.Millisecond {
			remain = 50 * time.Millisecond
		}
		_ = conn.SetReadDeadline(time.Now().Add(remain))
		chunk := make([]byte, 4096)
		n, err := conn.Read(chunk)
		if n > 0 {
			buf = append(buf, chunk[:n]...)
		}
		if err != nil && n == 0 {
			conn.Close()
			return nil, errors.New("ws handshake closed")
		}
	}
	idx := strings.Index(string(buf), "\r\n\r\n")
	header, rest := buf[:idx], buf[idx+4:]
	statusLine := string(header)
	if i := strings.Index(statusLine, "\r\n"); i >= 0 {
		statusLine = statusLine[:i]
	}
	if !strings.Contains(statusLine, "101") {
		conn.Close()
		return nil, errors.New("ws handshake failed: " + statusLine)
	}
	_ = conn.SetReadDeadline(time.Time{})
	return &miniWS{conn: conn, buf: append([]byte(nil), rest...), nextID: 1}, nil
}

func cdpCall(w *miniWS, method string, params map[string]any, timeout time.Duration) map[string]any {
	msg, _ := cdpExchange(w, method, params, timeout)
	return msg
}

func cdpExchange(w *miniWS, method string, params map[string]any, timeout time.Duration) (map[string]any, bool) {
	w.mu.Lock()
	msgID := w.nextID
	w.nextID++
	w.mu.Unlock()
	payload := map[string]any{"id": msgID, "method": method}
	if params != nil {
		payload["params"] = params
	}
	raw, _ := json.Marshal(payload)
	if err := w.sendText(string(raw)); err != nil {
		return nil, false
	}
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		remain := time.Until(deadline)
		if remain < 200*time.Millisecond {
			remain = 200 * time.Millisecond
		}
		opcode, data, err := w.recvMessage(remain)
		if err != nil {
			if errors.Is(err, errWSClosed) {
				return nil, false
			}
			continue
		}
		if opcode != 0x1 && opcode != 0x2 {
			continue
		}
		var msg map[string]any
		if err := json.Unmarshal(data, &msg); err != nil {
			continue
		}
		if id, ok := py.NumOK(msg["id"]); ok && int(id) == msgID {
			return msg, true
		}
	}
	return nil, true
}

func jsonWithAccessToken(raw string) map[string]any {
	cur := strings.TrimSpace(raw)
	if cur == "" {
		return nil
	}
	for i := 0; i < 3; i++ {
		if strings.Contains(cur, "access_token") {
			var obj map[string]any
			if err := json.Unmarshal([]byte(cur), &obj); err == nil && obj != nil && truthy(obj["access_token"]) {
				return obj
			}
		}
		nxt := pyUnquote(cur)
		if nxt == cur {
			break
		}
		cur = nxt
	}
	return nil
}

func pyUnquote(s string) string {
	if !strings.Contains(s, "%") {
		return s
	}
	out := make([]byte, 0, len(s))
	for i := 0; i < len(s); {
		if s[i] == '%' && i+2 < len(s) {
			hi, ok1 := unhex(s[i+1])
			lo, ok2 := unhex(s[i+2])
			if ok1 && ok2 {
				out = append(out, hi<<4|lo)
				i += 3
				continue
			}
		}
		out = append(out, s[i])
		i++
	}
	return string(out)
}

func unhex(c byte) (byte, bool) {
	switch {
	case '0' <= c && c <= '9':
		return c - '0', true
	case 'a' <= c && c <= 'f':
		return c - 'a' + 10, true
	case 'A' <= c && c <= 'F':
		return c - 'A' + 10, true
	}
	return 0, false
}

func truthy(v any) bool {
	switch x := v.(type) {
	case nil:
		return false
	case string:
		return x != ""
	case bool:
		return x
	case float64:
		return x != 0
	case []any:
		return len(x) > 0
	case map[string]any:
		return len(x) > 0
	}
	return true
}

func cookiesFromDocumentCookie(text string) []map[string]any {
	var cookies []map[string]any
	for _, part := range strings.Split(text, ";") {
		part = strings.TrimSpace(part)
		if part == "" || !strings.Contains(part, "=") {
			continue
		}
		name, value, _ := strings.Cut(part, "=")
		cookies = append(cookies, map[string]any{"name": strings.TrimSpace(name), "value": value})
	}
	return cookies
}

func tokensFromCookieList(cookies []map[string]any) map[string]any {
	if len(cookies) == 0 {
		return nil
	}
	body := map[string]any{}
	var oauth map[string]any
	wssdi := ""
	for _, c := range cookies {
		if c == nil {
			continue
		}
		name := py.S(c["name"])
		value := py.S(c["value"])
		if name == DeviceCookie && value != "" {
			wssdi = value
		}
		if parsed := jsonWithAccessToken(value); parsed != nil {
			if name == OAuthCookie || oauth == nil {
				oauth = parsed
			}
		}
	}
	if oauth == nil || !truthy(oauth["access_token"]) {
		return nil
	}
	for _, k := range []string{"access_token", "refresh_token", "identity_canonical_id", "client_id", "session_id"} {
		if truthy(oauth[k]) {
			body[k] = oauth[k]
		}
	}
	if ident := ws.IdentityFrom(oauth); ident != "" {
		body["identity_canonical_id"] = ident
	}
	if v, ok := oauth["expires_at"]; ok && v != nil {
		body["expires_at"] = v
	}
	if wssdi != "" {
		body["wssdi"] = wssdi
	}
	return body
}

func cdpCookieList(msg map[string]any) []map[string]any {
	res, _ := msg["result"].(map[string]any)
	raw, _ := res["cookies"].([]any)
	out := make([]map[string]any, 0, len(raw))
	for _, c := range raw {
		m, _ := c.(map[string]any)
		out = append(out, m)
	}
	return out
}

func (a *App) cdpCookiesFromTarget(wsURL string) map[string]any {
	w, err := wsConnect(wsURL, secs(captureCallSec))
	if err != nil {
		return nil
	}
	defer w.close()
	ua := ""
	if ver := cdpCall(w, "Browser.getVersion", nil, secs(captureCallSec)); ver != nil {
		if res, ok := ver["result"].(map[string]any); ok {
			ua = py.Strip(py.S(res["userAgent"]))
		}
	}
	if ua != "" {
		a.files.SaveUserAgent(ua)
	}
	cdpCall(w, "Network.enable", nil, secs(captureCallSec))
	cookies := cdpCookieList(cdpCall(w, "Network.getAllCookies", nil, secs(captureCallSec)))
	if body := tokensFromCookieList(cookies); body != nil {
		if ua != "" {
			body["user_agent"] = ua
		}
		return body
	}
	if extra := cdpCookieList(cdpCall(w, "Storage.getCookies", nil, secs(captureCallSec))); len(extra) > 0 {
		cookies = append(cookies, extra...)
	}
	if body := tokensFromCookieList(cookies); body != nil {
		if ua != "" {
			body["user_agent"] = ua
		}
		return body
	}
	val := ""
	if ev := cdpCall(w, "Runtime.evaluate", map[string]any{"expression": "document.cookie", "returnByValue": true}, secs(captureCallSec)); ev != nil {
		outer, _ := ev["result"].(map[string]any)
		inner, _ := outer["result"].(map[string]any)
		val = py.S(inner["value"])
	}
	body := tokensFromCookieList(cookiesFromDocumentCookie(val))
	if body != nil && ua != "" {
		body["user_agent"] = ua
	}
	return body
}

func (a *App) tryCaptureFromCDP(port int) map[string]any {
	var pages, others []map[string]any
	for _, t := range cdpList(port, time.Second) {
		if t == nil || py.S(t["webSocketDebuggerUrl"]) == "" {
			continue
		}
		if py.S(t["type"]) == "page" {
			pages = append(pages, t)
		} else {
			others = append(others, t)
		}
	}
	for _, t := range append(pages, others...) {
		body := a.cdpCookiesFromTarget(py.S(t["webSocketDebuggerUrl"]))
		if body != nil && truthy(body["access_token"]) {
			return body
		}
	}
	return nil
}

func cdpPages(port int) []map[string]any {
	var out []map[string]any
	for _, t := range cdpList(port, secs(windowCheckSec)) {
		if t != nil && py.S(t["type"]) == "page" && py.S(t["id"]) != "" {
			out = append(out, t)
		}
	}
	return out
}

type browserProc struct {
	cmd  *exec.Cmd
	done chan struct{}
	code int
}

func startBrowser(args []string) (*browserProc, error) {
	cmd := exec.Command(args[0], args[1:]...)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = nil, nil, nil
	detachProcess(cmd)
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	p := &browserProc{cmd: cmd, done: make(chan struct{})}
	go func() {
		err := cmd.Wait()
		if err != nil {
			if ee, ok := err.(*exec.ExitError); ok {
				p.code = ee.ExitCode()
			} else {
				p.code = -1
			}
		}
		close(p.done)
	}()
	return p, nil
}

func (p *browserProc) running() bool {
	if p == nil {
		return false
	}
	select {
	case <-p.done:
		return false
	default:
		return true
	}
}

func (p *browserProc) waitFor(d time.Duration) bool {
	if p == nil {
		return true
	}
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-p.done:
		return true
	case <-t.C:
		return false
	}
}

func (p *browserProc) pid() int {
	if p == nil || p.cmd.Process == nil {
		return 0
	}
	return p.cmd.Process.Pid
}

func (a *App) attemptIs(attempt int) bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	return attempt < 0 || a.state.loginAttempt == attempt
}

func (a *App) capturing() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	return a.state.capturing
}

func (a *App) captureLoop(proc *browserProc, debugPort int, attempt int) {
	if flag(os.Getenv("BAGHOLDER_LOGIN_NO_POLL")) {
		a.logf("bagholder login: cookie polling off; the window is not touched over DevTools\n")
		return
	}
	var refused any
	for a.attemptIs(attempt) {
		if !a.capturing() {
			return
		}
		var body map[string]any
		if len(cdpPages(debugPort)) > 0 {
			body = a.tryCaptureFromCDP(debugPort)
		}
		if body != nil && truthy(body["access_token"]) && a.attemptIs(attempt) && body["refresh_token"] != refused {
			if !a.capturing() {
				return
			}
			if truthy(a.captureTokens(body)["ok"]) {
				a.logf("bagholder captured Wealthsimple session\n")
				a.closeLoginBrowser(proc)
				return
			}
			refused = body["refresh_token"]
			a.logf("bagholder login: Wealthsimple refused the captured session on refresh; still watching the window\n")
		}
		time.Sleep(secs(captureEverySec))
	}
}

func (a *App) pollChromeSession(proc *browserProc, debugPort int, attempt int) {
	deadline := time.Now().Add(CaptureWaitSec * time.Second)
	start := time.Now()
	seenPage := false
	go a.captureLoop(proc, debugPort, attempt)
	for time.Now().Before(deadline) {
		if !a.attemptIs(attempt) {
			return
		}
		if !a.capturing() {
			return
		}
		var pages []map[string]any
		if proc.running() {
			pages = cdpPages(debugPort)
		}
		seenPage = seenPage || len(pages) > 0
		gone := !proc.running() || (len(pages) == 0 && (seenPage || time.Since(start) > 10*time.Second))
		if gone {
			if !a.attemptIs(attempt) {
				return
			}
			a.mu.Lock()
			if a.state.capturing {
				a.state.err = "The Chrome window closed before a session showed up. Choose Connect Wealthsimple to try again."
				a.state.capturing = false
			}
			a.mu.Unlock()
			a.logf("bagholder login: window closed, waiting stopped\n")
			a.closeLoginBrowser(proc)
			return
		}
		time.Sleep(secs(windowCheckSec))
	}
	if !a.attemptIs(attempt) {
		return
	}
	a.mu.Lock()
	if a.state.capturing {
		a.state.err = "No session yet. Finish login in the Chrome window, then wait a few seconds."
		a.state.capturing = false
	}
	a.mu.Unlock()
	a.closeLoginBrowser(proc)
}

func loginBrowserWS() string {
	raw, err := cdpGet(DebugPorts[0], "/json/version", 2*time.Second)
	if err != nil {
		return ""
	}
	var v map[string]any
	if err := json.Unmarshal(raw, &v); err != nil {
		return ""
	}
	return py.S(v["webSocketDebuggerUrl"])
}

func (a *App) closeLoginBrowser(only *browserProc) {
	a.mu.Lock()
	proc := a.state.chrome
	current := true
	if only != nil && proc != only {
		proc = only
		current = false
	}
	if current {
		a.state.chrome = nil
	}
	a.mu.Unlock()
	if proc == nil {
		return
	}
	if !current {
		if !proc.waitFor(100 * time.Millisecond) {
			terminateProcess(proc.cmd)
		}
		return
	}
	if wsURL := loginBrowserWS(); wsURL != "" {
		if w, err := wsConnect(wsURL, 5*time.Second); err == nil {
			cdpCall(w, "Browser.close", nil, 8*time.Second)
			w.close()
		}
	}
	if !proc.waitFor(5 * time.Second) {
		terminateProcess(proc.cmd)
	}
}

func (a *App) loginBrowserAlive() bool {
	a.mu.Lock()
	proc := a.state.chrome
	a.mu.Unlock()
	if proc == nil || !proc.running() || loginBrowserWS() == "" {
		return false
	}
	return len(cdpPages(DebugPorts[0])) > 0
}

func (a *App) loginViewDrop() {
	a.viewMu.Lock()
	w := a.view
	a.view, a.viewTarget = nil, ""
	a.viewMu.Unlock()
	if w != nil {
		w.close()
	}
}

func (a *App) loginViewWS() *miniWS {
	pages := cdpPages(DebugPorts[0])
	if len(pages) == 0 {
		a.loginViewDrop()
		return nil
	}
	page := pages[0]
	a.viewMu.Lock()
	if a.view != nil && a.viewTarget == py.S(page["id"]) {
		w := a.view
		a.viewMu.Unlock()
		return w
	}
	a.viewMu.Unlock()
	a.loginViewDrop()
	w, err := wsConnect(py.S(page["webSocketDebuggerUrl"]), secs(captureCallSec))
	if err != nil {
		return nil
	}
	a.viewMu.Lock()
	a.view, a.viewTarget = w, py.S(page["id"])
	a.viewMu.Unlock()
	return w
}

func (a *App) shotDrop() {
	a.shotMu.Lock()
	w := a.shot
	a.shot, a.shotTarget = nil, ""
	a.shotMu.Unlock()
	if w != nil {
		w.close()
	}
}

func (a *App) shotWS() *miniWS {
	pages := cdpPages(DebugPorts[0])
	if len(pages) == 0 {
		a.shotDrop()
		return nil
	}
	page := pages[0]
	a.shotMu.Lock()
	if a.shot != nil && a.shotTarget == py.S(page["id"]) {
		w := a.shot
		a.shotMu.Unlock()
		return w
	}
	a.shotMu.Unlock()
	a.shotDrop()
	w, err := wsConnect(py.S(page["webSocketDebuggerUrl"]), secs(captureCallSec))
	if err != nil {
		return nil
	}
	a.shotMu.Lock()
	a.shot, a.shotTarget = w, py.S(page["id"])
	a.shotMu.Unlock()
	return w
}

func (a *App) loginFrame() []byte {
	if !a.capturing() {
		return nil
	}
	w := a.shotWS()
	if w == nil {
		return nil
	}
	r := cdpCall(w, "Page.captureScreenshot", map[string]any{"format": "jpeg", "quality": 60}, secs(captureCallSec))
	if r == nil {
		a.shotDrop()
		return nil
	}
	res, _ := r["result"].(map[string]any)
	data := py.S(res["data"])
	if data == "" {
		return nil
	}
	raw, err := base64.StdEncoding.DecodeString(data)
	if err != nil {
		a.shotDrop()
		return nil
	}
	return raw
}

type screencast struct {
	mu    sync.Mutex
	cond  *sync.Cond
	frame []byte
	seq   int
}

func (a *App) publishFrame(frame []byte) {
	if len(frame) == 0 {
		return
	}
	a.cast.mu.Lock()
	a.cast.frame = frame
	a.cast.seq++
	a.cast.cond.Broadcast()
	a.cast.mu.Unlock()
}

func (a *App) shotLoop(attempt int) {
	last := -1
	for a.attemptIs(attempt) {
		if !a.capturing() {
			return
		}
		a.cast.mu.Lock()
		seq := a.cast.seq
		a.cast.mu.Unlock()
		if seq == last {
			a.publishFrame(a.loginFrame())
		}
		a.cast.mu.Lock()
		last = a.cast.seq
		a.cast.mu.Unlock()
		time.Sleep(shotEverySec)
	}
}

func safeURL(raw string) string {
	u, err := url.Parse(raw)
	if err != nil || u.Host == "" {
		if i := strings.IndexAny(raw, "?#"); i >= 0 {
			return raw[:i]
		}
		return raw
	}
	return u.Scheme + "://" + u.Host + u.Path
}

func (a *App) loginTraceLoop(attempt int) {
	if !flag(os.Getenv("BAGHOLDER_LOGIN_DEBUG")) {
		return
	}
	for a.attemptIs(attempt) {
		if !a.capturing() {
			return
		}
		pages := cdpPages(DebugPorts[0])
		if len(pages) == 0 {
			time.Sleep(500 * time.Millisecond)
			continue
		}
		w, err := wsConnect(py.S(pages[0]["webSocketDebuggerUrl"]), secs(captureCallSec))
		if err != nil {
			time.Sleep(500 * time.Millisecond)
			continue
		}
		cdpCall(w, "Page.enable", nil, secs(captureCallSec))
		cdpCall(w, "Network.enable", nil, secs(captureCallSec))
		cdpCall(w, "Log.enable", nil, secs(captureCallSec))
		cdpCall(w, "Runtime.enable", nil, secs(captureCallSec))
		sent := map[string]string{}
		for a.attemptIs(attempt) {
			if !a.capturing() {
				w.close()
				return
			}
			opcode, data, err := w.recvMessage(2 * time.Second)
			if err != nil {
				if errors.Is(err, errWSTimeout) {
					continue
				}
				break
			}
			if opcode != 0x1 && opcode != 0x2 {
				continue
			}
			var msg map[string]any
			if json.Unmarshal(data, &msg) != nil {
				break
			}
			p, _ := msg["params"].(map[string]any)
			switch py.S(msg["method"]) {
			case "Page.frameNavigated":
				fr, _ := p["frame"].(map[string]any)
				if py.S(fr["parentId"]) == "" {
					a.logf("bagholder login: page %s\n", safeURL(py.S(fr["url"])))
				}
			case "Network.requestWillBeSent":
				req, _ := p["request"].(map[string]any)
				if id := py.S(p["requestId"]); id != "" {
					if len(sent) > 256 {
						sent = map[string]string{}
					}
					sent[id] = safeURL(py.S(req["url"]))
				}
			case "Network.loadingFailed":
				id := py.S(p["requestId"])
				a.logf("bagholder login: request failed (%s) %s %s\n", py.S(p["type"]), py.S(p["errorText"]), sent[id])
				delete(sent, id)
			case "Log.entryAdded":
				e, _ := p["entry"].(map[string]any)
				if py.S(e["level"]) == "error" {
					a.logf("bagholder login: page error %s\n", py.S(e["text"]))
				}
			}
		}
		w.close()
		time.Sleep(500 * time.Millisecond)
	}
}

func (a *App) screencastLoop(attempt int) {
	for a.attemptIs(attempt) {
		if !a.capturing() {
			return
		}
		pages := cdpPages(DebugPorts[0])
		if len(pages) == 0 {
			time.Sleep(500 * time.Millisecond)
			continue
		}
		w, err := wsConnect(py.S(pages[0]["webSocketDebuggerUrl"]), secs(captureCallSec))
		if err != nil {
			time.Sleep(500 * time.Millisecond)
			continue
		}
		cdpCall(w, "Page.startScreencast", map[string]any{"format": "jpeg", "quality": 60, "maxWidth": LoginViewSize[0], "maxHeight": LoginViewSize[1], "everyNthFrame": 1}, secs(captureCallSec))
		failed := false
		for a.attemptIs(attempt) && !failed {
			if !a.capturing() {
				w.close()
				return
			}
			opcode, data, err := w.recvMessage(2 * time.Second)
			if err != nil {
				if errors.Is(err, errWSTimeout) {
					continue
				}
				failed = true
				break
			}
			if opcode != 0x1 && opcode != 0x2 {
				continue
			}
			var msg map[string]any
			if err := json.Unmarshal(data, &msg); err != nil {
				failed = true
				break
			}
			if py.S(msg["method"]) != "Page.screencastFrame" {
				continue
			}
			p, _ := msg["params"].(map[string]any)
			frame, _ := base64.StdEncoding.DecodeString(py.S(p["data"]))
			a.publishFrame(frame)
			w.mu.Lock()
			id := w.nextID
			w.nextID++
			w.mu.Unlock()
			ack, _ := json.Marshal(map[string]any{"id": id, "method": "Page.screencastFrameAck", "params": map[string]any{"sessionId": p["sessionId"]}})
			if err := w.sendText(string(ack)); err != nil {
				failed = true
			}
		}
		w.close()
		if failed {
			time.Sleep(500 * time.Millisecond)
		}
	}
}

func (a *App) loginStream(write func([]byte) error, alive func() bool) {
	last := -1
	for {
		if !a.capturing() {
			return
		}
		a.cast.mu.Lock()
		if a.cast.seq == last {
			timer := time.AfterFunc(time.Second, func() {
				a.cast.mu.Lock()
				a.cast.cond.Broadcast()
				a.cast.mu.Unlock()
			})
			a.cast.cond.Wait()
			timer.Stop()
		}
		if a.cast.seq == last || a.cast.frame == nil {
			a.cast.mu.Unlock()
			continue
		}
		frame, seq := a.cast.frame, a.cast.seq
		a.cast.mu.Unlock()
		last = seq
		if !alive() {
			return
		}
		head := []byte("--frame\r\nContent-Type: image/jpeg\r\nContent-Length: " + strconv.Itoa(len(frame)) + "\r\n\r\n")
		if err := write(append(append(head, frame...), '\r', '\n')); err != nil {
			return
		}
	}
}

var viewKeys = map[string]int{"Enter": 13, "Tab": 9, "Backspace": 8, "Delete": 46, "Escape": 27, "ArrowLeft": 37, "ArrowUp": 38, "ArrowRight": 39, "ArrowDown": 40, "Home": 36, "End": 35}

func viewKeyEvent(ch rune) map[string]any {
	s := string(ch)
	up := strings.ToUpper(s)
	code, vk := "", 0
	switch {
	case unicode.IsDigit(ch):
		code, vk = "Digit"+s, int(ch)
	case ch < 128 && up >= "A" && up <= "Z" && len(up) == 1:
		code, vk = "Key"+up, int(up[0])
	case ch == ' ':
		code, vk = "Space", 32
	}
	ev := map[string]any{"key": s, "text": s, "unmodifiedText": s, "code": code}
	if vk != 0 {
		ev["windowsVirtualKeyCode"] = vk
		ev["nativeVirtualKeyCode"] = vk
	}
	return ev
}

func isAlnum(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if !unicode.IsLetter(r) && !unicode.IsDigit(r) {
			return false
		}
	}
	return true
}

func withType(ev map[string]any, typ string) map[string]any {
	out := make(map[string]any, len(ev)+1)
	for k, v := range ev {
		out[k] = v
	}
	out["type"] = typ
	return out
}

func (a *App) loginInput(ev map[string]any) map[string]any {
	kind := py.S(ev["kind"])
	w := a.loginViewWS()
	if w == nil {
		return map[string]any{"ok": false, "error": "No login window."}
	}
	x, y := py.Num(ev["x"], 0), py.Num(ev["y"], 0)
	failed := false
	call := func(method string, params map[string]any) {
		if _, live := cdpExchange(w, method, params, secs(captureCallSec)); !live {
			failed = true
		}
	}
	switch kind {
	case "click":
		call("Input.dispatchMouseEvent", map[string]any{"type": "mouseMoved", "x": x, "y": y})
		for _, typ := range []string{"mousePressed", "mouseReleased"} {
			call("Input.dispatchMouseEvent", map[string]any{"type": typ, "x": x, "y": y, "button": "left", "clickCount": 1})
		}
	case "text":
		text := py.S(ev["text"])
		runes := []rune(text)
		if len(runes) == 1 || (len(runes) > 0 && len(runes) <= 8 && isAlnum(text)) {
			for _, ch := range runes {
				call("Input.dispatchKeyEvent", withType(viewKeyEvent(ch), "keyDown"))
				call("Input.dispatchKeyEvent", withType(viewKeyEvent(ch), "keyUp"))
			}
		} else if text != "" {
			call("Input.insertText", map[string]any{"text": text})
		}
	case "key":
		key := py.S(ev["key"])
		vk, ok := viewKeys[key]
		if !ok {
			return map[string]any{"ok": false, "error": "unknown key"}
		}
		base := map[string]any{"key": key, "code": key, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk}
		if key == "Enter" {
			base["text"] = "\r"
		}
		call("Input.dispatchKeyEvent", withType(base, "keyDown"))
		call("Input.dispatchKeyEvent", withType(base, "keyUp"))
	case "wheel":
		call("Input.dispatchMouseEvent", map[string]any{"type": "mouseWheel", "x": x, "y": y, "deltaX": 0, "deltaY": py.Num(ev["deltaY"], 0)})
	default:
		return map[string]any{"ok": false, "error": "unknown input"}
	}
	if failed {
		a.loginViewDrop()
		return map[string]any{"ok": false, "error": "The login window did not take that."}
	}
	return map[string]any{"ok": true}
}

func (a *App) cancelLogin() map[string]any {
	a.mu.Lock()
	was := a.state.capturing
	a.state.capturing = false
	a.state.err = ""
	a.mu.Unlock()
	a.logf("bagholder login: cancelled\n")
	a.closeLoginBrowser(nil)
	return map[string]any{"ok": true, "cancelled": was}
}

func (a *App) startLoginBrowser() map[string]any {
	a.logf("bagholder login: connect requested\n")
	if a.loginBrowserAlive() {
		if wsURL := loginBrowserWS(); wsURL != "" {
			if w, err := wsConnect(wsURL, 5*time.Second); err == nil {
				if pages := cdpPages(DebugPorts[0]); len(pages) > 0 {
					cdpCall(w, "Target.activateTarget", map[string]any{"targetId": pages[0]["id"]}, 8*time.Second)
				}
				w.close()
			}
		}
		a.mu.Lock()
		already := a.state.capturing
		a.state.capturing = true
		a.state.err = ""
		proc := a.state.chrome
		if !already {
			a.state.loginAttempt++
		}
		attempt := a.state.loginAttempt
		a.mu.Unlock()
		if !already {
			go a.pollChromeSession(proc, DebugPorts[0], attempt)
		}
		a.logf("bagholder login: window already up, brought forward\n")
		return map[string]any{"ok": true, "reused": true}
	}
	a.closeLoginBrowser(nil)
	chrome := FindChrome()
	if chrome == "" {
		return map[string]any{"ok": false, "error": BrowserMissing}
	}
	profile := filepath.Join(a.cfg.Home, "chrome")
	_ = ensureHome(a.cfg.Home)
	_ = os.MkdirAll(profile, 0o700)
	debugPort := DebugPorts[0]
	args := []string{chrome, "--user-data-dir=" + profile, "--remote-debugging-port=" + strconv.Itoa(debugPort), "--remote-debugging-address=127.0.0.1", "--remote-allow-origins=http://127.0.0.1", "--no-first-run", "--no-default-browser-check", "--new-window"}
	if a.cfg.LoginView {
		args = append(args, "--no-sandbox", "--disable-gpu", "--disable-dev-shm-usage", "--window-position=0,0", fmt.Sprintf("--window-size=%d,%d", LoginViewSize[0], LoginViewSize[1]))
	}
	args = append(args, LoginURL)
	proc, err := startBrowser(args)
	if err != nil {
		return map[string]any{"ok": false, "error": BrowserMissing}
	}
	a.logf("bagholder login: chrome launched (pid %d)\n", proc.pid())
	a.mu.Lock()
	a.state.chrome = proc
	a.state.capturing = true
	a.state.err = ""
	a.state.loginAttempt++
	attempt := a.state.loginAttempt
	a.mu.Unlock()
	go a.pollChromeSession(proc, debugPort, attempt)
	if a.cfg.LoginView {
		a.cast.mu.Lock()
		a.cast.frame, a.cast.seq = nil, 0
		a.cast.mu.Unlock()
		go a.screencastLoop(attempt)
		go a.shotLoop(attempt)
		go a.loginTraceLoop(attempt)
	}
	return map[string]any{"ok": true}
}

func (a *App) captureTokens(body map[string]any) map[string]any {
	if body == nil {
		return map[string]any{"ok": false, "error": "bad body"}
	}
	if !truthy(body["access_token"]) {
		return map[string]any{"ok": false, "error": "missing access_token"}
	}
	sess := a.loadSession()
	if sess == nil {
		sess = ws.Session{}
	}
	for _, k := range []string{"access_token", "refresh_token", "identity_canonical_id", "expires_at", "wssdi", "client_id", "session_id", "user_agent"} {
		if truthy(body[k]) {
			sess[k] = body[k]
		}
	}
	ident := ws.IdentityFrom(body)
	if ident == "" {
		ident = ws.IdentityFrom(sess)
	}
	info := map[string]any{}
	if sess.Str("access_token") != "" {
		info = a.ws.TokenInfo(sess)
	}
	if ident == "" {
		ident = ws.IdentityFrom(info)
	}
	if ident != "" {
		sess["identity_canonical_id"] = ident
	}
	if sess.Str("session_id") == "" {
		sess["session_id"] = py.UUID4()
	}
	a.ws.ApplyTokenInfoClientID(sess, info)
	if sess.Str("client_id") == "" {
		if cid := a.ws.ScrapeClientID(); cid != "" {
			sess["client_id"] = cid
		}
	}
	if sess.Str("user_agent") == "" {
		if ua := a.files.CachedUserAgent(); ua != "" {
			sess["user_agent"] = ua
		}
	}
	if !a.ws.RefreshSession(sess, false) {
		a.mu.Lock()
		a.state.connected = false
		err := a.state.err
		if err == "" {
			err = "Wealthsimple refused the captured login"
		}
		a.mu.Unlock()
		return map[string]any{"ok": false, "error": err}
	}
	a.saveSession(sess)
	a.mu.Lock()
	a.state.connected = true
	a.state.capturing = false
	a.state.err = ""
	a.mu.Unlock()
	go a.runSync(true, true)
	return map[string]any{"ok": true}
}
